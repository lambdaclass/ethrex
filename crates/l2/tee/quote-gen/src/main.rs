use configfs_tsm::OpenQuote;
use dcap_qvl::collateral::get_collateral_from_pcs;
use dcap_qvl::verify::verify;

#[tokio::main]
async fn main() {
    let mut quote = OpenQuote::new(&"test").unwrap();

    // Assert that the provider must be TDX
    quote.check_provider(vec!["tdx_guest"]).unwrap();

    // Give 64 null bytes as input data
    quote.write_input([0; 64]).unwrap();

    let output = quote.read_output().unwrap();
    println!("Quote: {:?}", output);
    println!("Generation: {}", quote.read_generation().unwrap());

    let collateral = get_collateral_from_pcs(&output, std::time::Duration::from_secs(10)).await.expect("failed to get collateral");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let tcb = verify(&output, &collateral, now).expect("failed to verify quote");
    println!("{:?}", tcb);
}
