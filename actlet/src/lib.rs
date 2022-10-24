use std::any::type_name;
use std::future::Future;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[cfg(feature = "derive")]
pub use actlet_derive::Message;

/// Errors that may happen in communication with an actor.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to send a message")]
    Send,
    #[error("Failed to receive a reply")]
    Recv,
}

type Result<T> = std::result::Result<T, Error>;

/// An actor system.
pub struct System {
    /// An address of a promoter.
    addr: Address<promoter::Promoter>,
    /// A cancellation token used for gracefully stopping the actor system.
    stop_token: CancellationToken,
}

impl System {
    /// Create an actor system.
    pub fn new() -> Self {
        let stop_token = CancellationToken::new();
        let addr = promoter::spawn(stop_token.child_token());
        System { addr, stop_token }
    }

    /// Invoke gracefully stopping the actor system.
    ///
    /// This function doesn't wait for all tasks spawned by the system getting
    /// stopped.
    pub fn stop(self) {
        self.stop_token.cancel();
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

    fn spawn_task<F>(&self, fut: F) -> JoinHandle<()>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let stop_token = self.stop_token.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = fut => (),
                _ = stop_token.cancelled() => (),
            }
        })
    }
}

/// An actor execution context.
pub struct Context<A> {
    own_addr: Address<A>,
    promoter_addr: Address<promoter::Promoter>,
    stop_token: CancellationToken,
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

    fn spawn_task<F>(&self, fut: F) -> JoinHandle<()>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let stop_token = self.stop_token.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = fut => (),
                _ = stop_token.cancelled() => (),
            }
        })
    }
}

/// An address of an actor.
pub struct Address<A> {
    sender: mpsc::Sender<Box<dyn Dispatch<A> + Send>>,
}

impl<A> Address<A> {
    const MAX_MESSAGES: usize = 256;

    fn pair() -> (Self, mpsc::Receiver<Box<dyn Dispatch<A> + Send>>) {
        let (sender, receiver) = mpsc::channel(Self::MAX_MESSAGES);
        let addr = Address { sender };
        (addr, receiver)
    }
}

impl<A> Clone for Address<A> {
    fn clone(&self) -> Self {
        Address {
            sender: self.sender.clone(),
        }
    }
}

impl<A, M> Into<Caller<M>> for Address<A>
where
    A: Handler<M>,
    M: Action,
    // An message will be converted into `Box<dyn Dispatch>`.
    M: 'static,
{
    fn into(self) -> Caller<M> {
        Caller::new(self)
    }
}

impl<A, M> Into<Emitter<M>> for Address<A>
where
    A: Handler<M>,
    M: Signal,
    // An message will be converted into `Box<dyn Dispatch>`.
    M: 'static,
{
    fn into(self) -> Emitter<M> {
        Emitter::new(self)
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
                    tracing::error!("{} stopped", type_name::<A>());
                    Err(Error::Recv)
                }
            },
            Err(_) => {
                tracing::error!("{} stopped", type_name::<A>());
                Err(Error::Send)
            }
        }
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
            tracing::warn!("{} stopped", type_name::<A>());
        }
    }

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
                tokio::spawn(async move {
                    if let Err(_) = sender.send(dispatcher).await {
                        tracing::warn!("{} stopped", type_name::<A>());
                    }
                });
            }
            Err(TrySendError::Closed(_)) => {
                tracing::warn!("{} stopped", type_name::<A>());
            }
        }
    }
}

/// A type that implements [`Call<M>`] for a particular message.
pub struct Caller<M> {
    inner: Box<dyn Call<M> + Send + Sync>,
}

impl<M> Caller<M>
where
    M: Action,
{
    pub fn new<T>(inner: T) -> Self
    where
        T: Call<M> + Send + Sync,
        // `T` will be converted into Box<dyn Call<M>>`.
        T: 'static,
    {
        Caller {
            inner: Box::new(inner),
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
pub struct Emitter<M> {
    inner: Box<dyn Emit<M> + Send + Sync>,
}

impl<M> Emitter<M>
where
    M: Signal,
{
    pub fn new<T>(inner: T) -> Self
    where
        T: Emit<M> + Send + Sync,
        // `T` will be converted into Box<dyn Call<M>>`.
        T: 'static,
    {
        Emitter {
            inner: Box::new(inner),
        }
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

    fn fire(&self, msg: M) {
        self.inner.fire(msg);
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
    async fn started(&mut self, ctx: &mut Context<Self>) {}

    /// Called when the actor stopped.
    #[allow(unused_variables)]
    async fn stopped(&mut self, ctx: &mut Context<Self>) {}
}

/// A trait to spawn a new asynchronous task.
#[async_trait]
pub trait Spawn: Sized {
    /// Spawns a new asynchronous task dedicated for an actor.
    async fn spawn_actor<A>(&self, actor: A) -> Address<A>
    where
        A: Actor;

    /// Spawns a new asynchronous task dedicated for a `Future`.
    fn spawn_task<F>(&self, fut: F) -> JoinHandle<()>
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

/// A trait that every message sent by [`Emit<M>`] must implement.
pub trait Signal: Message<Reply = ()> {}

/// A trait to send every message.
#[async_trait]
pub trait Emit<M: Signal> {
    /// Sends a message.
    ///
    /// The `msg` will be lost if the actor has already stopped.
    async fn emit(&self, msg: M);

    /// Sends a message synchronously if possible.
    ///
    /// This function is useful when a message has to be sent outside `async fn`
    /// and `async` blocks such as `Drop::drop()`.
    ///
    /// The `msg` will be lost if the actor has already stopped.
    #[allow(unused_variables)]
    fn fire(&self, msg: M) {
        unimplemented!("Emit::fire");
    }
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
}

impl<A: Actor> MessageLoop<A> {
    fn new(
        actor: A,
        receiver: mpsc::Receiver<Box<dyn Dispatch<A> + Send>>,
        context: Context<A>,
    ) -> Self {
        MessageLoop {
            actor,
            receiver,
            context,
        }
    }

    async fn run(&mut self) {
        self.actor.started(&mut self.context).await;
        let stop_token = self.context.stop_token.clone();
        loop {
            tokio::select! {
                Some(dispatch) = self.receiver.recv() => {
                    dispatch.dispatch(&mut self.actor, &mut self.context).await;
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
}

impl<M> ActionDispatcher<M>
where
    M: Action,
{
    fn new(msg: M, sender: oneshot::Sender<M::Reply>) -> Self {
        ActionDispatcher {
            message: Some(msg),
            sender: Some(sender),
        }
    }
}

#[async_trait]
impl<A, M> Dispatch<A> for ActionDispatcher<M>
where
    A: Handler<M>,
    M: Action,
{
    async fn dispatch(mut self: Box<Self>, actor: &mut A, ctx: &mut Context<A>) {
        let message = self.message.take().unwrap();
        let sender = self.sender.take().unwrap();
        let reply = actor.handle(message, ctx).await;
        if sender.send(reply).is_err() {
            tracing::error!("Failed to send, {} stopped", type_name::<A>());
        }
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

mod promoter {
    use super::*;

    pub(crate) struct Promoter;

    pub(crate) fn spawn(stop_token: CancellationToken) -> Address<Promoter> {
        let (addr, receiver) = Address::pair();
        let context = Context::new(addr.clone(), addr.clone(), stop_token);
        tokio::spawn(async move {
            MessageLoop::new(Promoter::new(), receiver, context)
                .run()
                .await;
        });
        addr
    }

    impl Promoter {
        fn new() -> Self {
            Promoter
        }

        fn spawn<A>(&self, actor: A, ctx: &mut Context<Self>) -> Address<A>
        where
            A: Actor,
        {
            let (addr, receiver) = Address::pair();
            let ctx = Context::new(
                addr.clone(),
                ctx.address().clone(),
                ctx.stop_token.child_token(),
            );
            tokio::spawn(async move {
                MessageLoop::new(actor, receiver, ctx).run().await;
            });
            addr
        }
    }

    #[async_trait]
    impl Actor for Promoter {
        async fn started(&mut self, _ctx: &mut Context<Self>) {
            tracing::debug!(event = "started");
        }

        async fn stopped(&mut self, _ctx: &mut Context<Self>) {
            tracing::debug!(event = "stopped");
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
