# [DRAFT] Ethrex roadmap for becoming based

> [!NOTE]
> This document is still under development, and everything stated in it is subject to change.
> Feedback is more than welcome.

> [!IMPORTANT]
> This initial approach is **decentralized** and **permissionless** but not **based** yet. Although sequencing rights aren't currently guaranteed to the L1 proposer, there will be incentives for L1 proposers to eventually participate in the L2, moving toward [Justin Drake's definition](https://ethresear.ch/t/based-rollups-superpowers-from-l1-sequencing/15016).

From the beginning, [ethrex](https://github.com/lambdaclass/ethrex) was conceived not just as an Ethereum L1 client, but also as an L2 (ZK Rollup). This means anyone will be able to use ethrex to deploy an EVM-equivalent, multi-prover (supporting SP1, RISC Zero, and TEEs) based rollup with just one command. We recently wrote a [blog post](https://blog.lambdaclass.com/celebrating-a-year-of-ethrex/) where we expand this idea more in depth.

The purpose of this document is to provide a high-level overview of how ethrex will implement its based rollup feature.

## State of the art

While Ethereum Foundation members are actively discussing and proposing EIPs to integrate based sequencing into Ethereum and efforts to coordinate and standardize various components needed for based rollups- like [FABRIC](https://ethresear.ch/t/fabric-fabric-to-accelerate-based-rollup-infrastructure-connectivity/21640) proposes. The following table offers a high-level comparison of the based sequencing approaches before we present our proposal.

> [!NOTE]
> This table compares the different based rollups in the ecosystem based on their current development state, not their final form.

| Based Rollup       | Protocol       | Sequencer Election | Proof System                    | Preconfs                |  Additional Context  |
| ------------------ | -------------- | ------------------ | ------------------------------- | ----------------------- | --- |
| Taiko Alethia      | Permissioned | -      | Multi-proof (sgxGeth (TEE), and sgxReth (ZK/TEE)) | Yes |  -  |
| Based OP (Gattaca) | Permissioned   | Round Robin        | Single Proof (optimistic)       | Yes                     |   For phase 1, the Sequencer/Gateway is centralized. For phase 2 (current phase) the Sequencer/Gateway is permissioned.  |
| Spire              | Permissionless | Dutch Auction      | Single Proof (optimistic)       | Yes                     |  -   |
| R1                | Permissionless | Total Anarchy      | Multi-proof (ZK, TEE, Guardian) | No                      |  R1 is yet to be specified but planned to be built on top of Surge and Taiko's Stack. They're waiting until Taiko is mature enough to have preconfs   |
| Surge            | Permissionless | Total Anarchy      | Multi-proof (ZK, TEE, Guardian) | No                      |  Surge is built on top of Taiko Alethia but it's tuned enough to be a Stage 2 rollup. They're waiting until Taiko is mature enough to have preconfs. |

Other based rollups not mentioned will be added later.

## Ethrex proposal for based sequencing

According to Justin Drake's definition of "based", being "based" implies that the L1 proposers are the ones who, at the end of the day, sequence the L2, either personally or by delegating the responsibility to a third party.

However, today, the "based" ecosystem is very immature. Despite the constant efforts of various teams, no stack is fully prepared to meet this definition. Additionally, L1 proposers do not have sufficient economic incentives to be part of the protocol.

But there's a way out. As mentioned in Spire's [What is a based rollup?](https://docs.spire.dev/education-hub/what-is-a-based-rollup)

> The key to this definition is that sequencing is "driven" by a base layer and not controlled by a completely external party.

Following this, our proposal's main focus is **decentralization** and **low operation cost**, and we don't want to sacrifice them in favor of preconfirmations or composability.

Considering this, after researching existing approaches, we concluded that a decentralized, permissionless ticket auction is the most practical first step for ethrex's based sequencing solution.

Ultimately, we aim to align with [Gattaca's model for based sequencing](https://ethresear.ch/t/becoming-based-a-path-towards-decentralised-sequencing/21733) and collaborate with [FABRIC](https://ethresear.ch/t/fabric-fabric-to-accelerate-based-rollup-infrastructure-connectivity/21640) efforts to standardize based rollups and helping interoperability.

[Rogue](https://x.com/fede_intern/status/1846035499799978475) and many upcoming rollups will be using this solution from the beginning.

## Benefits of our approach

The key benefits of our approach to based sequencing are:

- **Decentralization and Permissionlessness from the Get-Go:** We've decentralized ethrex L2 by allowing anyone to participate in the L2 block proposal; actors willing to participate on it can do this permissionlessly, as the execution ticket auction approach we are taking provides a governance free leader election mechanism.
- **Robust Censorship Resistance:** By being decentralized and permissionless, and with the addition of Sequencer challenges, we increased the cost of censorship in the protocol.
- **Low Operational Cost:** We strived to make the sequencer operating costs as low as possible by extending the sequencing window, allowing infrequent L1 finalization for low traffic periods.

## Key points

### Terminology

- **Ticket:** non-transferable right of a Sequencer to build and commit an L2 batch. One or more are auctioned during each **auction period**.
- **Sequencing Period:** the period during which a ticket holder has sequencing rights.
- **Auction Period:** the period during which the auction is performed.
<!-- TODO: Allocated? Allocation? Auctioned? -->
- **Allocated Period:** the set of **contiguous sequencing periods** allocated among the winners **of the corresponding auctioning period** -during an auctioning period, multiple sequencing periods are auctioned, the set of these is the allocated period.
- **L2 batch:** A collection of L2 blocks submitted to L1 in a single transaction.
- **Commit Transaction:** An L1 transaction submitted by the Lead Sequencer to commit to an L2 batch execution.
- **Sequencer:** An L2 node registered in the designated L1 contract.
- **Lead Sequencer:** The Sequencer currently authorized to build L2 blocks and post L2 batches during a specific L1 block.
- **Follower:** Non-Lead Sequencer nodes, which may be Sequencers awaiting leadership or passive nodes.

### How it will work

As outlined earlier, sequencing rights for future blocks are allocated through periodic ticket auctions. To participate, sequencers must register and provide collateral. Each auction occurs during a designated auction period, which spans a defined range of L1 blocks. These auctions are held a certain number of blocks in advance of the allocated period.

During each auction period, a configurable number of tickets are auctioned off. Each ticket grants its holder the right to sequence transactions during one sequencing period within the allocated period. However, at the time of the auction, the specific sequencing period assigned to each ticket remains undetermined. Once the auction period ends, the sequencing periods are randomly assigned (shuffled) among the ticket holders, thereby determining which sequencing period each ticket corresponds to.

<!-- TODO: add updated graph -->

1. Sequencers individually opt in before auction period `n` ends, providing collateral via an L1 contract. This registration is a one-time process per Sequencer.
2. During the auction, registered Sequencers bid for sequencing rights for a yet-to-be-revealed sequencing period within the allocated period.
3. At the auction's conclusion, sequencing rights for the sequencing periods within the allocated period are assigned among the ticket holders.
4. Finally, Sequencers submit L2 batch transactions to L1 during their assigned sequencing period (note: this step does not immediately follow step 3, as additional auctions and sequencing might occur in-between).

To ensure L2 liveness in this decentralized protocol, Sequencers must participate in a peer-to-peer (P2P) network. The diagram below illustrates this process:

![Diagram showing the end-to-end flow of a transaction in the ethrex L2 P2P layer](./l2_p2p_diagram.png)

1. A User: sends a transaction to the network.
2. Any node: Gossips in the P2P a received transaction. So every transaction lives in a public distributed mempool
3. The Lead Sequencer: Produces an L2 block including that transaction.
4. The Lead Sequencer: Broadcasts the L2 block, including the transaction, to the network via P2P.
5. Any node: Executes the block, gossips it, and keeps its state up to date.
6. The Lead Sequencer: Seals the batch in L2.
7. The Lead Sequencer: Posts the batch to the L1 in a single transaction.
8. The Lead Sequencer: Broadcasts the "batch sealed" message to the network via P2P.
9. Any node: Seals the batch locally and gossips the message.
10. A User: Receives a non-null receipt for the transaction.

Lead Sequencers will follow the following pipeline for L2 block building and batch commitment:

<!-- TODO: add updated graph with L1 block time distribution -->

## Protocol details

<!-- TODO: add this section -->

## Downsides

Below we list some of the risks and known issues we are aware of that this protocol introduces. Some of them were highlighted thanks to the feedback of different teams that took the time to review our first draft.

- **Inconsistent UX:** If a Sequencer fails to include its batch submit transaction in the L1, the blocks it contains will simply be reorged out once the first batch of the next sequencer is published. Honest sequencers can avoid this by not building new batches some slots before their turn ends. The next Sequencer can, in turn, start building their first batch earlier to avoid dead times. This is similar to Taiko’s permissioned network, where sequencers coordinate to stop proposing 4 slots before their turn ends to avoid reorgs.
- **Batch Stealing:** Lead Sequencers that fail to publish their batches before their sequencing period ends might have their batches "stolen" by the next Lead Sequencer, which can republish those batches as their own. We can mitigate in the same way as the last point.
- **Long Finalization Times:** Since publishing batches to L1 is infrequent, users might experience long finalization times during low traffic periods. We can solve this by assuming a transaction in an L2 block transmitted through P2P will eventually be published to L1, and punishing Sequencers that don't include some of their blocks in a batch.
- **Temporary Network Blinding:** A dishonest Sequencer may blind the network if they don't gossip blocks nor publish the batches to the L1 as part of the commit transactions' calldata. While the first case alone is mitigated through an L1 syncing mechanism, if the necessary data to sync is not available we can't rely on it. In this case, the prover ensures this doesn't happen by requiring the batch as a public input to the proof verification. That way, the bad batch can't be verified, and will be reverted.
- **High-Fee Transactions Hoarding:** A dishonest Sequencer might not share high-fee transactions with the Lead Sequencer with the hope of processing them once it's their turn to be Lead Sequencer. This is a non-issue, since transaction senders can simply propagate their transaction themselves, either by sending it to multiple RPC providers, or to their own node.
- **Front-running and Sandwiching Attacks:** Lead Sequencers have the right to reorder transactions as they like and we expect they'll use this to extract MEV, including front-running and sandwiching attacks, which impact user experience. We don't have plans to address this at the protocol level, but we expect solutions to appear at the application level, same as in L1.
- **No Sequencers Scenario:** If a sequencing period has no elected Lead Sequencer, we establish Full Anarchy during that period, so anyone can advance the chain. This is a last resort, and we don't expect this happening in practice.

## Conclusion

To preserve decentralization and permissionlessness, we chose ticket auctions for leader election, at the expense of preconfirmations and composability.

As mentioned at the beginning, this approach does not fully align with [Justin Drake's definition](https://ethresear.ch/t/based-rollups-superpowers-from-l1-sequencing/15016) of "based" rollups but is "based enough" to serve as a starting point. Although the current design cannot guarantee that sequencing rights are assigned exclusively to the L1 proposer for each slot, we're interested in achieving this, and will do so once the conditions are met, namely, that L1 proposer lookahead is available.

So what about "based" Ethrex tomorrow? Eventually, there will be enough incentives for L1 proposers to either run their own L2 Sequencers or delegate their L1 rights to an external one. At that stage, the auction and assignment of L2 sequencing rights will be linked to the current L1 proposer or their delegated Sequencer. Periods may also adjust as lookahead tables, such as the [Deterministic Lookahead Proposal](https://eips.ethereum.org/EIPS/eip-7917) or [RAID](https://eth-fabric.github.io/website/research/raid), become viable.

This proposal is intentionally minimalistic and adaptable for future refinements. How this will change and adapt to future necessities is something we don't know right now, and we don't care about it until those necessities arrive; this is [Lambda's engineering philosophy](https://blog.lambdaclass.com/lambdas-engineering-philosophy/).

## Open questions

- Do we want this protocol to incentivize L1 proposers to join? How are we doing that? If we guarantee registered L1 proposers to sequence during their slot, what would happen with the rest of the sequencing period? If they don't pay for sequencing, why would others?

## Further considerations

The following are things we are looking to tackle in the future, but which are not blockers for our current work.

- Ticket Pricing Strategies.
- Delegation Processes.
- Preconfirmations.
- Bonding.
- L1 Reorgs Handling.

## References and acknowledgements

The following links, repos, and projects have been important in the development of this document, we have learned a lot from them and want to thank and acknowledge them.

### Context

- [Stages of a Rollup](https://medium.com/l2beat/introducing-stages-a-framework-to-evaluate-rollups-maturity-d290bb22befe)
- [PBS](https://ethereum.org/en/roadmap/pbs/)
- [Total Anarchy](https://vitalik.eth.limo/general/2021/01/05/rollup.html)
- [FABRIC](https://ethresear.ch/t/fabric-fabric-to-accelerate-based-rollup-infrastructure-connectivity/21640)

### Intro to based rollups

- [Based Rollups by Justin Drake (current accepted definition)](https://ethresear.ch/t/based-rollups-superpowers-from-l1-sequencing/15016)
- [Based Rollups by Spire](https://docs.spire.dev/education-hub/what-is-a-based-rollup)
- [Based Rollups by Taiko](https://docs.taiko.xyz/taiko-alethia-protocol/protocol-design/based-rollups/)
- [Based Rollups by Gattaca](https://ethresear.ch/t/becoming-based-a-path-towards-decentralised-sequencing/21733)
  - [Analysis on Gattaca's Based Rollup Approach by Spire](https://docs.spire.dev/education-hub/based-rollups-overview)

### Based rollups benefits

- [Based Preconfirmations](https://ethresear.ch/t/based-preconfirmations/17353)

### Based rollups + extra steps

- [Based Ticketing Rollup by George Spasov](https://hackmd.io/@Perseverance/Syk2oQU36)
- [Based Contestable Rollup by Taiko (Taiko Alethia)](https://docs.taiko.xyz/taiko-alethia-protocol/protocol-design/contestable-rollup)
- [Native Based Rollup by Taiko (Taiko Gwyneth)](https://docs.taiko.xyz/taiko-gwyneth-protocol/what-is-taiko-gwyneth/)

### Misc

- [Why Total Anarchy Doesn't Pay the Bills](https://ethresear.ch/t/understanding-based-rollups-pga-challenges-total-anarchy-and-potential-solutions/21320)
- [Based Espresso: Based Sequencing for all L2s](https://hackmd.io/@EspressoSystems/BasedEspresso)

### Execution tickets

- [Execution Tickets](https://ethresear.ch/t/execution-tickets/17944)
- [Execution Tickets vs Execution Auctions](https://ethresear.ch/t/execution-auctions-as-an-alternative-to-execution-tickets/19894)
- [Economic Analysis of Execution Tickets](https://ethresear.ch/t/economic-analysis-of-execution-tickets/18894)
- [Beyond the Stars: An Introduction to Execution Tickets on Ethereum](https://www.ephema.io/blog/beyond-the-stars-an-introduction-to-execution-tickets-on-ethereum)

### Current based rollups

- [Rogue (LambdaClass)](https://x.com/fede_intern/status/1846035499799978475)
- [Surge (Nethermind)](https://github.com/NethermindEth/surge)
- [Taiko Alethia (Taiko Labs)](https://github.com/taikoxyz/taiko-mono)
- [Based OP (Gattaca + Lambdaclass)](https://github.com/gattaca-com/based-op)
- [R1](https://ethereumr1.org/)
- [Minimal Rollup (OpenZeppelin)](https://github.com/OpenZeppelin/minimal-rollup)

### Educational sources

- [FABRIC's list](https://eth-fabric.github.io/website/education)
- [Spire's list](https://docs.spire.dev/education-hub/based-resources)
