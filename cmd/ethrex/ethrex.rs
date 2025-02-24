mod cli;
mod decode;
mod launch;
mod networks;
mod utils;

#[tokio::main]
async fn main() {
    let matches = cli::cli().get_matches();
    cfg_if::cfg_if! {
        if #[cfg(feature = "l2")] {
            use launch::launch_l2;
            launch_l2(matches).await;
        } else {
            use launch::launch_l1;
            launch_l1(matches).await;
        }
    }
}
