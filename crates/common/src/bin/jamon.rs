use ethrex_common::parse_toml;

fn main() {
    match ethrex_common::parse_toml::read_toml() {
        Ok(_) => (),
        Err(err) => {
            panic!("{}", err);
        }
    };
}
