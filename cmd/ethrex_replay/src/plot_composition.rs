use ethrex_common::types::{Block, Transaction, TxKind};
use std::collections::HashMap;
use tracing::info;

use charming::{
    Chart, ImageRenderer,
    component::Legend,
    element::{Tooltip, Trigger},
    series::Pie,
};

const TOP_N_DESTINATIONS: usize = 10;
const TOP_N_SELECTORS: usize = 10;

#[derive(Default, Debug)]
struct BlockStats {
    destinations: HashMap<String, i64>,
    selector: HashMap<String, i64>,
    selector_by_gas: HashMap<String, i64>,
}

fn categorize_selector(sel: [u8; 4]) -> String {
    let selector = hex::encode(sel);
    match selector.as_str() {
        "a9059cbb" => "transfer",
        "095ea7b3" => "approve",
        "3593564c" => "swap", // execute(bytes,bytes[],uint256)
        "5f575529" => "swap",
        "2213bc0b" => "swap", // exec(address,address,uint256,address,bytes)
        "b6f9de95" => "swap",
        "1f6a1eb9" => "exec", // execute(bytes,bytes)
        "791ac947" => "swap",
        "23b872dd" => "transfer",
        "c46b30bc" => "swap",
        "0162e2d0" => "swap",
        "12aa3caf" => "swap",
        "78e111f6" => "mevbot", // executeFFsYo
        "088890dc" => "swap",
        "3d0e3ec5" => "swap",
        "9871efa4" => "swap",
        "c14c9204" => "oracle",
        "7ff36ab5" => "swap",
        "deff4b24" => "relay",
        "0d5f0e3b" => "swap",
        "b143044b" => "exec", // execute(ExecuteParam[])
        "d9caed12" => "withdraw",
        "6a761202" => "exec",
        "e63d38ed" => "mass transfer",
        "fb3bdb41" => "swap",
        "049639fb" => "exec",
        "d0e30db0" => "deposit",
        "18cbafe5" => "swap",
        "7b939232" => "deposit",
        "0894edf1" => "relay",       // commitVerification(bytes,bytes32)
        "28832cbd" => "swap&bridge", // swapAndStartBridgeTokensViaAcrossV3(...)
        "07ed2379" => "swap",
        "e9ae5c53" => "exec",
        "0cf79e0a" => "swap",
        "c7a76969" => "swap", // strictlySwapAndCallDln(...)
        "4782f779" => "withdraw",
        "2c65169e" => "swap", // buyWithEth(uint256,bool)
        "3ce33bff" => "bridge",
        "0dcd7a6c" => "transfer", // sendMultiSigToken(...)
        "2c57e884" => "swap",
        "38ed1739" => "swap",
        "b6b55f25" => "deposit",
        "3ccfd60b" => "withdraw",
        "30c48952" => "swap&bridge", // swapAndStartBridgeTokensViaMayan
        "13d79a0b" => "swap",        // settle
        _ => &selector,
    }
    .to_string()
}

impl BlockStats {
    fn process(&mut self, tx: Transaction) {
        if let TxKind::Call(addr) = tx.to() {
            *self.destinations.entry(format!("0x{addr:x}")).or_insert(0) += 1;
            if tx.data().len() >= 4 {
                let mut selector = [0u8; 4];
                selector.clone_from_slice(&tx.data()[0..4]);
                *self
                    .selector
                    .entry(categorize_selector(selector))
                    .or_insert(0) += 1;
                *self
                    .selector_by_gas
                    .entry(categorize_selector(selector))
                    .or_insert(0) += tx.gas_limit() as i64;
            }
        }
    }
    fn chart(&self) -> Vec<(String, Chart)> {
        let mut destinations: Vec<(_, _)> = self.destinations.iter().collect();
        destinations.sort_by_key(|(_, c)| -**c);
        let mut selectors: Vec<(_, _)> = self.selector.iter().collect();
        selectors.sort_by_key(|(_, c)| -**c);
        let mut selectors_by_gas: Vec<(_, _)> = self.selector_by_gas.iter().collect();
        selectors_by_gas.sort_by_key(|(_, c)| -**c);

        vec![
            (
                "selectors".to_string(),
                Chart::new()
                    .tooltip(Tooltip::new().trigger(Trigger::Item))
                    .legend(Legend::new())
                    .series(
                        Pie::new()
                            .name(format!("Top{TOP_N_SELECTORS} selectors"))
                            .radius(vec!["40%", "55%"])
                            .data(
                                truncate_to(selectors, TOP_N_SELECTORS)
                                    .iter()
                                    .map(|(name, count)| (*count as f64, name.as_str()))
                                    .collect(),
                            ),
                    ),
            ),
            (
                "selectors_by_gas".to_string(),
                Chart::new()
                    .tooltip(Tooltip::new().trigger(Trigger::Item))
                    .legend(Legend::new())
                    .series(
                        Pie::new()
                            .name(format!("Top{TOP_N_SELECTORS} selectors by gas limit"))
                            .radius(vec!["40%", "55%"])
                            .data(
                                truncate_to(selectors_by_gas, TOP_N_SELECTORS)
                                    .iter()
                                    .map(|(name, count)| (*count as f64, name.as_str()))
                                    .collect(),
                            ),
                    ),
            ),
            (
                "destinations".to_string(),
                Chart::new()
                    .tooltip(Tooltip::new().trigger(Trigger::Item))
                    .legend(Legend::new())
                    .series(
                        Pie::new()
                            .name(format!("Top{TOP_N_DESTINATIONS} destinations"))
                            .radius(vec!["40%", "55%"])
                            .data(
                                truncate_to(destinations, TOP_N_DESTINATIONS)
                                    .iter()
                                    .map(|(name, count)| (*count as f64, name.as_str()))
                                    .collect(),
                            ),
                    ),
            ),
        ]
    }
}

pub async fn plot(blocks: &[Block]) -> eyre::Result<()> {
    let mut stats = BlockStats::default();
    let txs = blocks
        .iter()
        .flat_map(|b| b.body.transactions.clone())
        .collect::<Vec<_>>();
    for tx in txs {
        stats.process(tx);
    }
    let first = blocks.first().map(|b| b.header.number).unwrap();
    let last = blocks.last().map(|b| b.header.number).unwrap();
    let mut renderer = ImageRenderer::new(1000, 800);
    for (name, chart) in stats.chart() {
        let filename = format!("chart_{name}_{first}-{last}.svg");
        info!("Saving chart to: {filename}");
        renderer.save(&chart, &filename)?;
    }
    Ok(())
}

fn truncate_to(vec: Vec<(&String, &i64)>, size: usize) -> Vec<(String, i64)> {
    let mut included = 0;
    let mut res: Vec<(String, i64)> = Vec::new();
    for (item, count) in vec.iter().take(size) {
        included += **count;
        res.push((item.to_string(), **count));
    }
    let total = vec.iter().map(|(_, c)| **c).sum::<i64>();
    res.push(("other".to_string(), total - included));
    res
}
