mod tools;

#[tokio::main]
async fn main() -> carmy::Result {
    // Every #[carmy::tool] in src/tools registers itself.
    // Dependencies go here: carmy::app().state(db).run().await
    carmy::run().await
}
