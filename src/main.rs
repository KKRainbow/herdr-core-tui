#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = herdr_tui::run(
        std::env::args().skip(1).collect(),
        std::collections::BTreeMap::new(),
    )
    .await
    {
        eprintln!("herdr-tui: {error}");
        std::process::exit(1);
    }
}
