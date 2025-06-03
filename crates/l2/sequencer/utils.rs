use rand::Rng;
use std::time::Duration;
use tokio::time::sleep;

pub async fn sleep_random(sleep_amount: u64) {
    sleep(random_duration(sleep_amount)).await;
}

pub fn random_duration(sleep_amount: u64) -> Duration {
    let random_noise: u64 = {
        let mut rng = rand::thread_rng();
        rng.gen_range(0..400)
    };
    Duration::from_millis(sleep_amount + random_noise)
}
