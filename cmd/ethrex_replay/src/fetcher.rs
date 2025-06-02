pub async fn or_latest(maybe_number: Option<usize>, rpc_url: &str) -> eyre::Result<usize> {
    Ok(match maybe_number {
        Some(v) => v,
        None => get_latest_block_number(rpc_url).await?,
    })
}

pub async fn get_blockdata(
    rpc_url: &str,
    chain_config: ChainConfig,
    block_number: usize,
) -> eyre::Result<Cache> {
    if let Ok(cache) = load_cache(block_number) {
        return Ok(cache);
    }
    let block = get_block(&rpc_url, block_number)
        .await
        .wrap_err("failed to fetch block")?;

    let parent_block_header = get_block(rpc_url, block_number - 1)
        .await
        .wrap_err("failed to fetch block")?
        .header;

    println!("populating rpc db cache");
    let witness = get_witness(&rpc_url, block_number)
        .await
        .wrap_err("Failed to get execution witness")?;

    let db = to_exec_db_from_witness(chain_config, &witness)
        .wrap_err("Failed to build prover db from execution witness")?;

    let cache = Cache {
        blocks: vec![block],
        parent_block_header,
        witness,
        chain_config,
        db,
    };
    write_cache(&cache).expect("failed to write cache");
    Ok(cache)
}

pub async fn get_rangedata(
    rpc_url: &str,
    chain_config: ChainConfig,
    from: usize,
    to: usize,
) -> eyre::Result<Cache> {
    if let Ok(cache) = load_cache_batch(from, to) {
        return Ok(cache);
    }
    let mut blocks = Vec::with_capacity(to - from);
    for block_number in from..=to {
        let data = get_blockdata(rpc_url, chain_config, block_number).await?;
        blocks.push(data);
    }
    let first_cache = blocks.first().ok_or(eyre::Error::msg("empty range"))?;
    let first_block = first_cache.blocks[0].clone();
    let rpc_db = RpcDB::new(rpc_url, chain_config, from - 1);
    let mut used: HashMap<Address, HashSet<H256>> = HashMap::new();
    for block_data in blocks.iter() {
        for account in block_data.db.accounts.keys() {
            used.entry(*account).or_default();
        }
        for (account, storage) in block_data.db.storage.iter() {
            let slots = used.entry(*account).or_default();
            slots.extend(storage.keys());
        }
    }
    let to_fetch: Vec<(Address, Vec<H256>)> = used
        .into_iter()
        .map(|(address, storages)| (address, storages.into_iter().collect()))
        .collect();
    rpc_db.load_accounts(&to_fetch).await?;
    let mut proverdb = rpc_db.to_exec_db(&first_block)?;
    proverdb.block_hashes = blocks
        .iter()
        .flat_map(|cache| cache.db.block_hashes.clone())
        .collect();
    for block_data in blocks.iter() {
        proverdb
            .state_proofs
            .1
            .extend(block_data.db.state_proofs.1.clone());
        for (account, proofs) in block_data.db.storage_proofs.iter() {
            let entry = proverdb.storage_proofs.entry(*account).or_default();
            entry.1.extend(proofs.1.clone());
        }
    }
    dedup_proofs(&mut proverdb.state_proofs.1);
    for (_, proofs) in proverdb.storage_proofs.iter_mut() {
        dedup_proofs(&mut proofs.1);
    }
    let cache = Cache {
        blocks: blocks.iter().map(|cache| cache.blocks[0].clone()).collect(),
        parent_block_header: first_cache.parent_block_header.clone(),
        db: proverdb,
    };
    write_cache_batch(&cache)?;
    Ok(cache)
}

fn dedup_proofs(proofs: &mut Vec<Vec<u8>>) {
    let mut seen: HashSet<Vec<u8>, RandomState> = HashSet::from_iter(proofs.drain(..));
    *proofs = seen.drain().collect();
}

pub fn to_exec_db_from_witness(
    chain_config: ChainConfig,
    witness: &ExecutionWitnessResult,
) -> Result<ethrex_vm::ProverDB, ProverDBError> {
    let mut code = HashMap::new();
    for witness_code in &witness.codes {
        code.insert(code_hash(witness_code), witness_code.clone());
    }

    let mut block_hashes = HashMap::new();

    let initial_state_hash = witness
        .block_headers
        .first()
        .expect("no headers?")
        .state_root;

    for header in witness.block_headers.iter() {
        block_hashes.insert(header.number, header.hash());
    }

    let mut initial_node = None;

    for node in witness.state.iter() {
        let x = Node::decode_raw(node).expect("invalid node");
        let hash = x.compute_hash().finalize();
        if hash == initial_state_hash {
            initial_node = Some(node.clone());
            break;
        }
    }

    let state_trie =
        Trie::from_nodes(initial_node.as_ref(), &witness.state).expect("failed to create trie");

    let mut storage_tries = HashMap::new();
    for (addr, nodes) in &witness.storage_tries {
        let hashed_address = hash_address(addr);
        let Some(encoded_state) = state_trie
            .get(&hashed_address)
            .expect("Failed to get from trie")
        else {
            // TODO re-explore this. When testing with hoodi this happened block 521990 an this continue fixed it
            continue;
        };

        let state =
            AccountState::decode(&encoded_state).expect("Failed to get state from encoded state");

        let mut initial_node = None;

        for node in nodes.iter() {
            let x = Node::decode_raw(node).expect("invalid node");
            let hash = x.compute_hash().finalize();
            if hash == state.storage_root {
                initial_node = Some(node);
                break;
            }
        }

        let storage_trie = Trie::from_nodes(initial_node, nodes).unwrap();

        storage_tries.insert(*addr, storage_trie);
    }

    let state_trie = Arc::new(Mutex::new(state_trie));
    let storage_tries = Arc::new(Mutex::new(storage_tries));

    Ok(ProverDB {
        code,
        block_hashes,
        chain_config,
        state_trie,
        storage_tries,
    })
}
