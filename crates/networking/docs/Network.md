# Network

The network crate handles the ethereum networking protocols. This involves:

-   [Discovery protocol](#discovery-protocol): built on top of udp and it is how we discover new nodes.
-   devP2P: sits on top of tcp and is where the actual blockchain information exchange happens.

Implementation follows the official spec which can be found [here](https://github.com/ethereum/devp2p/tree/master). Also, we've inspired in some [geth code](https://github.com/ethereum/go-ethereum/tree/master/p2p/discover).

## Discovery protocol

In the next section, we'll be looking at the discovery protocol (discv4 to be more specific) and the way we have it set up. There are many points for improvement and here we discuss some possible solutions to them.

At startup, the discovery server launches three concurrent tokio tasks:

-   The listen loop for incoming requests.
-   A revalidation loop to ensure peers remain responsive.
-   A recursive lookup loop to request new peers and keep our table filled.

Before starting these tasks, we run a [startup](#startup) process to connect to an array of initial nodes.

Before diving into what each task does, first, we need to understand how we are storing our nodes. Nodes are stored in an in-memory matrix which we call a [Kademlia table](https://github.com/lambdaclass/ethrex/blob/main/crates/net/kademlia.rs#L20-L23), though it isn't really a Kademlia table as we don't thoroughly follow the spec but we take it as a reference, you can read more [here](https://en.wikipedia.org/wiki/Kademlia). This table holds:

-   Our `node_id`: `node_id`s are derived from the public key. They are the 64 bytes starting from index 1 of the encoded pub key.
-   A vector of 256 `bucket`s which holds:
    -   `peers`: a vector of 16 elements of type `PeersData` where we save the node record and other related data that we'll see later.
    -   `replacements`: a vector of 16 elements of `PeersData` that are not connected to us, but we consider them as potential replacements for those nodes that have disconnected from us.

Peers are not assigned to any bucket but they are assigned based on its $0 \le \text{distance} \le 255$ to our `node_id`. Distance is defined by:

```rust
pub fn distance(node_id_1: H512, node_id_2: H512) -> usize {
    let hash_1 = Keccak256::digest(node_id_1);
    let hash_2 = Keccak256::digest(node_id_2);
    let xor = H256(hash_1.into()) ^ H256(hash_2.into());
    let distance = U256::from_big_endian(xor.as_bytes());
    distance.bits().saturating_sub(1)
}
```

### Startup

Before starting the server, we do a startup where we connect to an array of seeders or bootnodes. This involves:

-   Receiving bootnodes via CLI params
-   Inserting them into our table
-   Pinging them to notify our presence, so they acknowledge us.

This startup is far from being completed. The current state allows us to do basic tests and connections. Later, we want to do a real startup by first trying to connect to those nodes we were previously connected. For that, we'd need to store nodes on the database. If those nodes aren't enough to fill our table, then we also ping some bootnodes, which could be hardcoded or received through the cli. Current issues are opened regarding [startup](https://github.com/lambdaclass/ethrex/issues/398) and [nodes db](https://github.com/lambdaclass/ethrex/issues/454).

### Listen loop

The listen loop handles messages sent to our socket. The spec defines 6 types of messages:

-   **Ping**: Responds with a `pong` message. If the peer is not in our table we add it, if the corresponding bucket is already filled then we add it as a replacement for that bucket. If it was inserted we send a `ping from our end to get an endpoint proof.
-   **Pong**: Verifies that the `pong` corresponds to a previously sent `ping`, if so we mark the peer as proven.
-   **FindNodes**: Responds with a `neighbors` message that contains as many as the 16 closest nodes from the given target. A target is a pubkey provided by the peer in the message. The response can't be sent in one packet as it might exceed the discv4 max packet size. So we split it into different packets.
-   **Neighbors**: First we verify that we have sent the corresponding `find_node` message. If so, we receive the peers, store them, and ping them. Also, every [`find_node` request](https://github.com/lambdaclass/ethrex/blob/229ca0b316a79403412a917d04e3b95f579c56c7/crates/net/discv4.rs#L305-L314) may have a [tokio `Sender`](https://docs.rs/tokio/latest/tokio/sync/mpsc/struct.Sender.html) attached, if that is the case, we forward the nodes from the message through the channel. This becomes useful when waiting for a `find_node` response, [something we do in the lookups](https://github.com/lambdaclass/ethrex/blob/229ca0b316a79403412a917d04e3b95f579c56c7/crates/net/net.rs#L517-L570).
-   **ENRRequest**: currently not implemented see [here](https://github.com/lambdaclass/ethrex/issues/432).
-   **ENRResponse**: same as above.

### Re-validations

Re-validations are tasks that are implemented as intervals, that is: they run an action every `x` wherever unit of time (currently configured to run every 30 seconds). The current flow of re-validation is as follows

1. Every 30 seconds (by default) we ping the three least recently pinged peers: this may be fine now to keep simplicity, but we might prefer to choose three random peers instead to avoid the search which might become expensive as our buckets start to fill with more peers.
2. In the next iteration we check if they have answered
    - if they have: we increment the liveness field by one.
    - otherwise: we decrement the liveness by a third of its value.
3. If the liveness field is 0, we delete it and insert a new one from the replacements table.

Liveness checks are not part of the spec but are taken from geth, see [here](https://github.com/ethereum/go-ethereum/blob/master/p2p/discover/table_reval.go). This field is useful because it provides us with good criteria of which nodes are connected and we "trust" more. This trustiness is useful when deciding if we want to store this node in the database to use it as a future seeder or when establishing a connection in p2p.

Re-validations are another point of potential improvement. While it may be fine for now to keep simplicity at max, pinging the last recently pinged peers becomes quite expensive as the number of peers in the table increases. And it also isn't very "just" in selecting nodes so that they get their liveness increased so we trust them more and we might consider them as a seeder. A possible improvement could be:

-   Keep two lists: one for nodes that have already been pinged, and another one for nodes that have not yet been revalidated. Let's call the former "a" and the second "b".
-   In the beginning, all nodes would belong to "a" and whenever we insert a new node, they would be pushed to "a".
-   We would have two intervals: one for pinging "a" and another for pinging to nodes in "b". The "b" would be quicker, as no initial validation has been done.
-   When picking a node to ping, we would do it randomly, which is the best form of justice for a node to become trusted by us.
-   When a node from `b` responds successfully, we move it to `a`, and when one from `a` does not respond, we move it to `b`.

This improvement follows somewhat what geth does, see [here](https://github.com/ethereum/go-ethereum/blob/master/p2p/discover/table_reval.go).

### Recursive Lookups

Recursive lookups are as with re-validations implemented as intervals. Their current flow is as follows:

1. Every 30min we spawn three concurrent lookups: one closest to our pubkey and three others closest to randomly generated pubkeys.
2. Every lookup starts with the closest nodes from our table. Each lookup keeps track of:
    - Peers that have already been asked for nodes
    - Peers that have been already seen
    - Potential peers to query for nodes: a vector of up to 16 entries holding the closest peers to the pubkey. This vector is initially filled with nodes from our table.
3. We send a `find_node` to the closest 3 nodes (that we have not yet asked) from the pubkey.
4. We wait for the neighbors' response and push or replace those who are closer to the potential peers.
5. We select three other nodes from the potential peers vector and do the same until one lookup has no node to ask.

The way to do lookups aren't part of the spec. Our implementation aligns with geth approach, see [here](https://github.com/ethereum/go-ethereum/blob/master/p2p/discover/v4_udp.go#L282-L310).

### An example of how you might build a network

Finally, here is an example of how you could build a network and see how they connect each other:

We'll have three nodes: `a`, `b`, and `c`, we'll start `a`, then `b` setting `a` as a bootnode, and finally we'll start `c` with `b` as bootnode we should see that `c` connects to both `a` and `b` and so all the network should be connected.

**node a**:
`cargo run --bin ethrex --network test_data/kurtosis.json`

We get the `enode` by querying the node_info:
`curl http://localhost:8545 \
  -X POST \
  -H "Content-Type: application/json" \
  --data '{"jsonrpc":"2.0","method":"admin_nodeInfo","params":[],"id":1}'`

**node b**
We start a new server passing the `node_a` `enode` as bootnodes

```bash
cargo run --bin ethrex --network ./test_data/kurtosis.json --bootnodes=`NODE_A_ENODE` \
--authrpc.port=8552 --http.port=8546 --p2p.port=30305 --discovery.port=3036
```

**node c**
Finally, with `node_c` we connect to `node_b`. When the lookup runs, `node_c` should end up connecting to `node_a`:

```bash
 cargo run --bin ethrex --network ./test_data/kurtosis.json --bootnodes=`NODE_B_ENODE`" \
--authrpc.port=8553 --http.port=8547 --p2p.port=30308 --discovery.port=30310
```

You could also spawn nodes from other clients and it should work as well.
