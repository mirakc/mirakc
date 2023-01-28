use std::any::type_name;
use std::collections::HashMap;
use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;
use tracing::Span;

#[cfg(feature = "derive")]
pub use actlet_derive::Message;

/// Errors that may happen in communication with an actor.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Failed to send a message")]
    Send,
    #[error("Failed to receive a reply")]
    Recv,
}

pub type Result<T> = std::result::Result<T, Error>;

/// An actor system.
pub struct System {
    /// An address of a promoter.
    addr: Address<promoter::Promoter>,
    /// A cancellation token used for gracefully stopping the actor system.
    stop_token: CancellationToken,
    /// A span the actor system belongs to.
    span: Span,
}

impl System {
    /// Create an actor system.
    pub fn new() -> Self {
        Self::with_span(Span::current())
    }

    /// Create an actor system with a specified span.
    pub fn with_span(span: Span) -> Self {
        tracing::debug!("System started");
        let stop_token = CancellationToken::new();
        let addr = promoter::spawn(stop_token.child_token(), span.clone());
        System {
            addr,
            stop_token,
            span,
        }
    }

    /// Invoke gracefully stopping the actor system.
    ///
    /// This function doesn't wait for all tasks spawned by the system getting
    /// stopped.
    ///
    /// Use [`System::shutdown()`].
    pub fn stop(&self) {
        self.stop_token.cancel();
    }

    /// Gracefully shutdown the actor system.
    ///
    /// Unlike [`System::stop()`], this function waits for all tasks spawned by
    /// the system getting stopped.
    pub async fn shutdown(self) {
        self.stop();
        self.addr.wait().await;
        tracing::debug!("System stopped");
    }
}

#[async_trait]
impl Spawn for System {
    async fn spawn_actor<A>(&self, actor: A) -> Address<A>
    where
        A: Actor,
    {
        self.addr
            .call(promoter::Spawn(actor))
            .await
            .expect("Promoter died?")
    }

    fn spawn_task<F>(&self, fut: F) -> CancellationToken
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let stop_token = self.stop_token.child_token();
        let token = stop_token.clone();
        let task = async move {
            tokio::select! {
                _ = fut => (),
                _ = stop_token.cancelled() => (),
            }
        };
        tokio::spawn(task.instrument(self.span.clone()));
        token
    }
}

/// An actor execution context.
pub struct Context<A> {
    // An actor execution context owns an address of the actor and this prevents
    // the actor from stopping even if external modules release addresses of the
    // actor.
    own_addr: Address<A>,
    promoter_addr: Address<promoter::Promoter>,
    stop_token: CancellationToken,
    post_process: Option<Box<dyn Dispatch<A> + Send + Sync>>,
}

impl<A> Context<A> {
    fn new(
        own_addr: Address<A>,
        promoter_addr: Address<promoter::Promoter>,
        stop_token: CancellationToken,
    ) -> Self {
        Context {
            own_addr,
            promoter_addr,
            stop_token,
            post_process: None,
        }
    }

    /// Returns the address of the actor.
    pub fn address(&self) -> &Address<A> {
        &self.own_addr
    }

    /// Stops the actor.
    pub fn stop(&mut self) {
        self.stop_token.cancel();
    }

    /// Set a message to be handled just after sending a reply for the message
    /// currently handled, before dispatching any other messages.
    ///
    /// This can be used for avoiding some kind of deadlock in message handlers.
    /// #705 is a good example.  In this case, we can solve the deadlock by
    /// separating a process which causes the deadlock from others and
    /// performing it after sending a reply.
    pub fn set_post_process<M>(&mut self, msg: M)
    where
        A: Handler<M>,
        M: Signal + Send + Sync,
        // An message will be converted into `Box<dyn Dispatch>`.
        M: 'static,
    {
        assert!(self.post_process.is_none());
        self.post_process = Some(Box::new(SignalDispatcher::new(msg)));
    }
}

#[async_trait]
impl<A> Spawn for Context<A> {
    async fn spawn_actor<B>(&self, actor: B) -> Address<B>
    where
        B: Actor,
    {
        self.promoter_addr
            .call(promoter::Spawn(actor))
            .await
            .expect("Promoter died?")
    }

    fn spawn_task<F>(&self, fut: F) -> CancellationToken
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let stop_token = self.stop_token.child_token();
        let token = stop_token.clone();
        let task = async move {
            tokio::select! {
                _ = fut => (),
                _ = stop_token.cancelled() => (),
            }
        };
        // The context is available only in the message loop which is running
        // inside the actor system's span.
        tokio::spawn(task.in_current_span());
        token
    }
}

impl<A, M> CallerFactory<M> for Context<A>
where
    A: Handler<M>,
    M: Action,
    // An message will be converted into `Box<dyn Dispatch>`.
    M: 'static,
{
    fn caller(&self) -> Caller<M> {
        self.address().caller()
    }
}

impl<A, M> EmitterFactory<M> for Context<A>
where
    A: Handler<M>,
    M: Signal,
    // An message will be converted into `Box<dyn Dispatch>`.
    M: 'static,
{
    fn emitter(&self) -> Emitter<M> {
        self.address().emitter()
    }
}

impl<A, M> TriggerFactory<M> for Context<A>
where
    A: Handler<M>,
    M: Signal,
    // An message will be converted into `Box<dyn Dispatch>`.
    M: 'static,
{
    fn trigger(&self, msg: M) -> Trigger<M> {
        self.address().trigger(msg)
    }
}

/// An address of an actor.
pub struct Address<A> {
    sender: mpsc::Sender<Box<dyn Dispatch<A> + Send>>,
    // A child token of the Actor's stop token.
    // This cannot stop the Actor.
    stop_token: CancellationToken,
    // A token used to wait for the actor to stop.
    // TODO: Replace it with a more suitable type.
    wait_token: CancellationToken,
}

impl<A> Address<A> {
    const MAX_MESSAGES: usize = 256;

    pub fn is_available(&self) -> bool {
        !self.sender.is_closed()
    }

    pub async fn wait(&self) {
        self.wait_token.cancelled().await;
    }

    fn pair(
        stop_token: CancellationToken,
        wait_token: CancellationToken,
    ) -> (Self, mpsc::Receiver<Box<dyn Dispatch<A> + Send>>) {
        let (sender, receiver) = mpsc::channel(Self::MAX_MESSAGES);
        let addr = Address {
            sender,
            stop_token,
            wait_token,
        };
        (addr, receiver)
    }
}

impl<A: Actor> Address<A> {
    /// Inspect an actor.
    pub async fn inspect<F>(&self, inspect: F) -> Result<()>
    where
        F: FnOnce(&mut A) + Send,
        // The function will be sent to a task.
        F: 'static,
    {
        let msg = Inspect {
            inspect,
            _phantom: PhantomData,
        };
        self.call(msg).await
    }
}

impl<A> Clone for Address<A> {
    fn clone(&self) -> Self {
        Address {
            sender: self.sender.clone(),
            stop_token: self.stop_token.clone(),
            wait_token: self.wait_token.clone(),
        }
    }
}

#[async_trait]
impl<A, M> Call<M> for Address<A>
where
    A: Handler<M>,
    M: Action,
    // An message will be converted into `Box<dyn Dispatch>`.
    M: 'static,
{
    async fn call(&self, msg: M) -> Result<M::Reply> {
        let (sender, receiver) = oneshot::channel::<M::Reply>();
        let dispatcher = Box::new(ActionDispatcher::new(msg, sender));
        match self.sender.send(dispatcher).await {
            Ok(_) => match receiver.await {
                Ok(reply) => Ok(reply),
                Err(_) => {
                    tracing::error!(
                        actor = type_name::<A>(),
                        msg = type_name::<M>(),
                        "Recv failed"
                    );
                    Err(Error::Recv)
                }
            },
            Err(_) => {
                tracing::error!(
                    actor = type_name::<A>(),
                    msg = type_name::<M>(),
                    "Send failed"
                );
                Err(Error::Send)
            }
        }
    }
}

impl<A, M> CallerFactory<M> for Address<A>
where
    A: Handler<M>,
    M: Action,
    // An message will be converted into `Box<dyn Dispatch>`.
    M: 'static,
{
    fn caller(&self) -> Caller<M> {
        Caller::new(self.clone())
    }
}

#[async_trait]
impl<A, M> Emit<M> for Address<A>
where
    A: Handler<M>,
    M: Signal,
    // An message will be converted into `Box<dyn Dispatch>`.
    M: 'static,
{
    async fn emit(&self, msg: M) {
        let dispatcher = Box::new(SignalDispatcher::new(msg));
        if let Err(_) = self.sender.send(dispatcher).await {
            tracing::warn!(
                actor = type_name::<A>(),
                msg = type_name::<M>(),
                "The actor is stopped, the message is lost"
            );
        }
    }
}

impl<A, M> EmitterFactory<M> for Address<A>
where
    A: Handler<M>,
    M: Signal,
    // An message will be converted into `Box<dyn Dispatch>`.
    M: 'static,
{
    fn emitter(&self) -> Emitter<M> {
        Emitter::new(self.clone())
    }
}

#[async_trait]
impl<A, M> Fire<M> for Address<A>
where
    A: Handler<M>,
    M: Signal,
    // An message will be converted into `Box<dyn Dispatch>`.
    M: 'static,
{
    fn fire(&self, msg: M) {
        use mpsc::error::TrySendError;

        let dispatcher = Box::new(SignalDispatcher::new(msg));
        match self.sender.try_send(dispatcher) {
            Ok(_) => {
                // Succeeded to send synchronously.
            }
            Err(TrySendError::Full(dispatcher)) => {
                // Need sending using an async task.
                let sender = self.sender.clone();
                let stop_token = self.stop_token.clone();
                let task = async move {
                    tokio::select! {
                        result = sender.send(dispatcher) => {
                            if let Err(_) = result {
                                tracing::warn!(
                                    actor = type_name::<A>(),
                                    msg = type_name::<M>(),
                                    "The actor is stopped, the message is lost"
                                );
                            }
                        }
                        _ = stop_token.cancelled() => (),
                    }
                };
                tokio::spawn(task.in_current_span());
            }
            Err(TrySendError::Closed(_)) => {
                tracing::warn!(
                    actor = type_name::<A>(),
                    msg = type_name::<M>(),
                    "The actor is stopped, the message is lost"
                );
            }
        }
    }
}

impl<A, M> TriggerFactory<M> for Address<A>
where
    A: Handler<M>,
    M: Signal,
    // An message will be converted into `Arc<dyn Dispatch>`.
    M: 'static,
{
    fn trigger(&self, msg: M) -> Trigger<M> {
        Trigger::new(self.clone(), msg)
    }
}

/// A type that implements [`Call<M>`] for a particular message.
#[derive(Clone)]
pub struct Caller<M> {
    inner: Arc<dyn Call<M> + Send + Sync>,
}

impl<M> Caller<M>
where
    M: Action,
{
    pub fn new<T>(inner: T) -> Self
    where
        T: Call<M> + Send + Sync,
        // `T` will be converted into Arc<dyn Call<M>>`.
        T: 'static,
    {
        Caller {
            inner: Arc::new(inner),
        }
    }
}

#[async_trait]
impl<M> Call<M> for Caller<M>
where
    M: Action,
{
    async fn call(&self, msg: M) -> Result<M::Reply> {
        self.inner.call(msg).await
    }
}

/// A type that implements [`Emit<M>`] for a particular message.
#[derive(Clone)]
pub struct Emitter<M> {
    inner: Arc<dyn Emit<M> + Send + Sync>,
}

impl<M> Emitter<M>
where
    M: Signal,
{
    pub fn new<T>(inner: T) -> Self
    where
        T: Emit<M> + Send + Sync,
        // `T` will be converted into Arc<dyn Emit<M>>`.
        T: 'static,
    {
        Emitter {
            inner: Arc::new(inner),
        }
    }
}

impl<M> std::fmt::Debug for Emitter<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Emitter<{}>", std::any::type_name::<M>())
    }
}

#[async_trait]
impl<M> Emit<M> for Emitter<M>
where
    M: Signal,
{
    async fn emit(&self, msg: M) {
        self.inner.emit(msg).await
    }
}

/// A type that implements [`Fire<M>`] and emits an message only once when the
/// object is destroyed.
pub struct Trigger<M>
where
    M: Signal,
{
    inner: Box<dyn Fire<M> + Send + Sync>,
    msg: Option<M>,
}

impl<M> Trigger<M>
where
    M: Signal,
{
    pub fn new<T>(inner: T, msg: M) -> Self
    where
        T: Fire<M> + Send + Sync,
        // `T` will be converted into Box<dyn Fire<M>>`.
        T: 'static,
    {
        Trigger {
            inner: Box::new(inner),
            msg: Some(msg),
        }
    }
}

impl<M> Drop for Trigger<M>
where
    M: Signal,
{
    fn drop(&mut self) {
        self.inner.fire(self.msg.take().unwrap());
    }
}

/// A type that holds emitters.
pub struct EmitterRegistry<M> {
    emitters: HashMap<usize, Emitter<M>>,
    capacity: usize,
    next_id: usize,
}

impl<M> EmitterRegistry<M>
where
    M: Clone + Signal,
{
    /// Default capacity.
    pub const DEFAULT_CAPACITY: usize = 32;

    /// Creates with the default capacity.
    pub fn new() -> Self {
        Self::with_capacity(Self::DEFAULT_CAPACITY)
    }

    /// Creates with a specified capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        EmitterRegistry {
            emitters: HashMap::with_capacity(capacity),
            capacity,
            next_id: 1,
        }
    }

    /// Emits messages.
    pub async fn emit(&self, msg: M) {
        for emitter in self.emitters.values() {
            emitter.emit(msg.clone()).await
        }
    }

    /// Registers an emitter.
    pub fn register(&mut self, emitter: Emitter<M>) -> usize {
        if self.is_full() {
            tracing::error!(msg = type_name::<M>(), "No space to register emitter");
            return 0;
        }
        let id = self.unique_id();
        self.emitters.insert(id, emitter);
        id
    }

    /// Unregisters an emitter by ID.
    pub fn unregister(&mut self, id: usize) {
        assert!(self.emitters.contains_key(&id));
        self.emitters.remove(&id);
    }

    fn unique_id(&mut self) -> usize {
        assert!(!self.is_full());
        let limit = self.capacity + 1; // `id` starts from 1.
        loop {
            let id = self.next_id;
            self.next_id += 1;
            if self.next_id == limit {
                self.next_id = 1;
            }
            if !self.emitters.contains_key(&id) {
                return id;
            }
        }
    }

    fn is_full(&self) -> bool {
        assert!(self.emitters.len() <= self.capacity);
        self.emitters.len() == self.capacity
    }
}

impl<M> Default for EmitterRegistry<M>
where
    M: Clone + Signal,
{
    fn default() -> Self {
        Self::new()
    }
}

/// A message to stop an actor.
pub struct Stop;
impl Message for Stop {
    type Reply = ();
}
impl Signal for Stop {}

#[async_trait]
impl<A: Actor> Handler<Stop> for A {
    async fn handle(&mut self, _msg: Stop, ctx: &mut Context<Self>) {
        ctx.stop();
    }
}

// traits

/// A trait that every actor must implement.
#[async_trait]
pub trait Actor
where
    Self: Send + Sized,
    // An actor will be sent to a dedicated task created by `tokio::spawn()`.
    // And the following constraint is needed for the same reason as
    // `std::thread::spawn()`.
    Self: 'static,
{
    /// Called when the actor gets started running on a dedicated task.
    #[allow(unused_variables)]
    async fn started(&mut self, ctx: &mut Context<Self>) {
        tracing::debug!(actor = type_name::<Self>(), "Started");
    }

    /// Called when the actor stopped.
    #[allow(unused_variables)]
    async fn stopped(&mut self, ctx: &mut Context<Self>) {
        tracing::debug!(actor = type_name::<Self>(), "Stopped");
    }
}

/// A trait to spawn a new asynchronous task.
#[async_trait]
pub trait Spawn: Sized {
    /// Spawns a new asynchronous task dedicated for an actor.
    async fn spawn_actor<A>(&self, actor: A) -> Address<A>
    where
        A: Actor;

    /// Spawns a new asynchronous task dedicated for a `Future`.
    fn spawn_task<F>(&self, fut: F) -> CancellationToken
    where
        F: Future<Output = ()> + Send + 'static;
}

/// A trait that every message must implement.
pub trait Message: Send {
    /// The type of reply for this message.
    type Reply: Send;
}

/// A trait that every message sent by [`Call<M>`] must implement.
pub trait Action: Message {}

/// A trait to send a message and wait for its reply.
#[async_trait]
pub trait Call<M: Action> {
    /// Sends a message and waits for its reply.
    ///
    /// The `msg` will be lost if the actor has already stopped.
    async fn call(&self, msg: M) -> Result<M::Reply>;
}

/// A trait to create a caller.
pub trait CallerFactory<M>
where
    M: Action,
{
    fn caller(&self) -> Caller<M>;
}

/// A trait that every message sent by [`Emit<M>`] must implement.
pub trait Signal: Message<Reply = ()> {}

/// A trait to send a message.
#[async_trait]
pub trait Emit<M: Signal> {
    /// Sends a message.
    ///
    /// The `msg` will be lost if the actor has already stopped.
    async fn emit(&self, msg: M);
}

/// A trait to create an emitter.
pub trait EmitterFactory<M>
where
    M: Signal,
{
    fn emitter(&self) -> Emitter<M>;
}

/// A trait to send a message without blocking.
pub trait Fire<M: Signal> {
    /// Sends a message synchronously if possible.
    ///
    /// This function is useful when a message has to be sent outside `async fn`
    /// and `async` blocks such as `Drop::drop()`.
    ///
    /// The `msg` will be lost if the actor has already stopped.
    fn fire(&self, msg: M);
}

/// A trait to create a trigger.
pub trait TriggerFactory<M>
where
    M: Signal,
{
    fn trigger(&self, msg: M) -> Trigger<M>;
}

/// A trait to handle a message.
#[async_trait]
pub trait Handler<M>
where
    Self: Actor,
    M: Message,
{
    /// Performs a computation specified by a message and optionally returns a
    /// result of the computation.
    async fn handle(&mut self, msg: M, ctx: &mut Context<Self>) -> M::Reply;
}

// private types

struct MessageLoop<A> {
    actor: A,
    receiver: mpsc::Receiver<Box<dyn Dispatch<A> + Send>>,
    context: Context<A>,
    wait_token: CancellationToken,
}

impl<A: Actor> MessageLoop<A> {
    fn new(
        actor: A,
        receiver: mpsc::Receiver<Box<dyn Dispatch<A> + Send>>,
        context: Context<A>,
        wait_token: CancellationToken,
    ) -> Self {
        MessageLoop {
            actor,
            receiver,
            context,
            wait_token,
        }
    }

    async fn run(&mut self) {
        self.actor.started(&mut self.context).await;
        let stop_token = self.context.stop_token.clone();
        loop {
            tokio::select! {
                Some(dispatch) = self.receiver.recv() => {
                    dispatch.dispatch(&mut self.actor, &mut self.context).await;
                    if let Some(post_process) = self.context.post_process.take() {
                        post_process.dispatch(&mut self.actor, &mut self.context).await;
                    }
                }
                _ = stop_token.cancelled() => {
                    self.receiver.close();
                    break;
                }
                else => break,
            }
        }
        // Ensure that the remaining messages are processed before the stop.
        while let Some(dispatch) = self.receiver.recv().await {
            dispatch.dispatch(&mut self.actor, &mut self.context).await;
        }
        self.actor.stopped(&mut self.context).await;
        self.wait_token.cancel();
    }
}

#[async_trait]
trait Dispatch<A> {
    async fn dispatch(self: Box<Self>, actor: &mut A, ctx: &mut Context<A>);
}

struct ActionDispatcher<M>
where
    M: Action,
{
    message: Option<M>,
    sender: Option<oneshot::Sender<M::Reply>>,
    span: Span,
}

impl<M> ActionDispatcher<M>
where
    M: Action,
{
    fn new(msg: M, sender: oneshot::Sender<M::Reply>) -> Self {
        ActionDispatcher {
            message: Some(msg),
            sender: Some(sender),
            span: Span::current(),
        }
    }
}

#[async_trait]
impl<A, M> Dispatch<A> for ActionDispatcher<M>
where
    A: Handler<M>,
    M: Action,
{
    // Don't use `Span::enter()` in async functions.
    async fn dispatch(mut self: Box<Self>, actor: &mut A, ctx: &mut Context<A>) {
        let message = self.message.take().unwrap();
        let sender = self.sender.take().unwrap();
        let reply = actor
            .handle(message, ctx)
            .instrument(self.span.clone())
            .await;
        self.span.in_scope(|| {
            if sender.send(reply).is_err() {
                tracing::error!(
                    actor = type_name::<A>(),
                    msg = type_name::<M>(),
                    "Reply failed"
                );
            }
        });
    }
}

struct SignalDispatcher<M>
where
    M: Signal,
{
    message: Option<M>,
}

impl<M> SignalDispatcher<M>
where
    M: Signal,
{
    fn new(msg: M) -> Self {
        SignalDispatcher { message: Some(msg) }
    }
}

#[async_trait]
impl<A, M> Dispatch<A> for SignalDispatcher<M>
where
    A: Handler<M>,
    M: Signal,
{
    async fn dispatch(mut self: Box<Self>, actor: &mut A, ctx: &mut Context<A>) {
        let message = self.message.take().unwrap();
        actor.handle(message, ctx).await;
    }
}

/// An internal message to inspect an actor.
struct Inspect<A, F> {
    inspect: F,
    _phantom: PhantomData<A>,
}

impl<A, F> Message for Inspect<A, F>
where
    A: Actor,
    F: FnOnce(&mut A) + Send,
{
    type Reply = ();
}

impl<A, F> Action for Inspect<A, F>
where
    A: Actor,
    F: FnOnce(&mut A) + Send,
{
}

#[async_trait]
impl<A, F> Handler<Inspect<A, F>> for A
where
    A: Actor,
    F: FnOnce(&mut A) + Send + 'static,
{
    async fn handle(&mut self, msg: Inspect<A, F>, _ctx: &mut Context<Self>) {
        (msg.inspect)(self);
    }
}

mod promoter {
    use super::*;

    pub(crate) struct Promoter {
        system_span: Span,
        wait_tokens: Vec<CancellationToken>,
    }

    pub(crate) fn spawn(stop_token: CancellationToken, system_span: Span) -> Address<Promoter> {
        let wait_token = CancellationToken::new();
        let (addr, receiver) = Address::pair(stop_token.child_token(), wait_token.clone());
        let context = Context::new(addr.clone(), addr.clone(), stop_token);
        let task = {
            let span = system_span.clone();
            async move {
                MessageLoop::new(Promoter::new(span), receiver, context, wait_token)
                    .run()
                    .await;
            }
        };
        tokio::spawn(task.instrument(system_span));
        addr
    }

    impl Promoter {
        fn new(system_span: Span) -> Self {
            Promoter {
                system_span,
                wait_tokens: vec![],
            }
        }

        fn spawn<A>(&mut self, actor: A, ctx: &mut Context<Self>) -> Address<A>
        where
            A: Actor,
        {
            let stop_token = ctx.stop_token.child_token();
            let wait_token = self.create_wait_token();
            let (addr, receiver) = Address::pair(stop_token.child_token(), wait_token.clone());
            let ctx = Context::new(addr.clone(), ctx.address().clone(), stop_token);
            let task = async move {
                MessageLoop::new(actor, receiver, ctx, wait_token)
                    .run()
                    .await;
            };
            // This function called in the `ActionDispatcher::dispatch()`
            // where the `ActionDispatcher::span` has been entered.
            // However, every actor lives in the actor system.  So, the
            // main loop should be executed in the system span.
            tokio::spawn(task.instrument(self.system_span.clone()));
            addr
        }

        fn create_wait_token(&mut self) -> CancellationToken {
            let wait_token = CancellationToken::new();
            self.wait_tokens.push(wait_token.clone());
            wait_token
        }
    }

    #[async_trait]
    impl Actor for Promoter {
        async fn started(&mut self, _ctx: &mut Context<Self>) {
            tracing::debug!("Started");
        }

        async fn stopped(&mut self, _ctx: &mut Context<Self>) {
            tracing::debug!("Waiting for actors to stop...");
            let wait_tokens = std::mem::replace(&mut self.wait_tokens, vec![]);
            for wait_token in wait_tokens.into_iter() {
                // TODO: introduce id for each actor.
                wait_token.cancelled().await;
            }
            tracing::debug!("Stopped");
        }
    }

    pub(crate) struct Spawn<A: Actor>(pub A);
    impl<A: Actor> Message for Spawn<A> {
        type Reply = Address<A>;
    }
    impl<A: Actor> Action for Spawn<A> {}

    #[async_trait]
    impl<A> Handler<Spawn<A>> for Promoter
    where
        A: Actor,
    {
        async fn handle(
            &mut self,
            msg: Spawn<A>,
            ctx: &mut Context<Self>,
        ) -> <Spawn<A> as Message>::Reply {
            self.spawn(msg.0, ctx)
        }
    }
}

pub mod prelude {
    // types
    pub use crate::Address;
    pub use crate::Caller;
    pub use crate::Context;
    pub use crate::Emitter;
    pub use crate::EmitterRegistry;
    pub use crate::System;
    pub use crate::Trigger;

    // traits
    pub use crate::Action;
    pub use crate::Actor;
    pub use crate::Call;
    pub use crate::CallerFactory;
    pub use crate::Emit;
    pub use crate::EmitterFactory;
    pub use crate::Fire;
    pub use crate::Handler;
    pub use crate::Message;
    pub use crate::Signal;
    pub use crate::Spawn;
    pub use crate::TriggerFactory;

    // dependencies
    pub use async_trait::async_trait;
}
