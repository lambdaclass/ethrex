mod cli;
mod decode;
mod launch;
#[cfg(not(feature = "l2"))]
mod networks;
mod utils;

#[tokio::main]
async fn main() {
    let matches = cli::cli().get_matches();
    cfg_if::cfg_if! {
        if #[cfg(feature = "l2")] {
            launch::l2::launch(matches).await;
      } else {
            launch::l1::launch(matches).await;
        }
    }
}
