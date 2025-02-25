use ethrex_common::parse_toml;
use std::env;

fn main() {
    let toml_config = std::env::var("CONFIG_FILE").unwrap_or("../l2/config.toml".to_string());
    let args: Vec<String> = env::args().collect();

    if args.len() > 3 {
        panic!("Wrong number of arguments");
    }

    match ethrex_common::parse_toml::read_toml(toml_config) {
        Ok(_) => (),
        Err(err) => {
            panic!("{}", err);
        }
    };
}
