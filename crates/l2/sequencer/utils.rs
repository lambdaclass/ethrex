use rand::Rng;
use std::time::Duration;
use tokio::time::sleep;

pub async fn sleep_random(sleep_amount: u64) {
    let random_noise: u64 = {
        let mut rng = rand::thread_rng();
        rng.gen_range(0..400)
    };

    sleep(Duration::from_millis(sleep_amount + random_noise)).await;
}
