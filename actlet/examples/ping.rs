use actlet::prelude::*;

struct MyActor {
    count: usize,
}

impl MyActor {
    fn new(count: usize) -> Self {
        MyActor { count }
    }
}

#[async_trait]
impl Actor for MyActor {}

#[async_trait]
impl Handler<Ping> for MyActor {
    async fn handle(&mut self, msg: Ping, _ctx: &mut Context<Self>) -> usize {
        self.count += msg.0;
        self.count
    }
}

#[derive(Message)]
#[reply("usize")]
struct Ping(usize);

#[tokio::main]
async fn main() -> actlet::Result<()> {
    let system = System::new();
    let actor = MyActor::new(10);
    let addr = system.spawn_actor(actor).await;
    let result = addr.call(Ping(10)).await?;
    println!("RESULT: {}", result == 20);
    system.stop();
    Ok(())
}
