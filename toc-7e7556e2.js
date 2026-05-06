// Populate the sidebar
//
// This is a script, and not included directly in the page, to control the total size of the book.
// The TOC contains an entry for each page, so if each page includes a copy of the TOC,
// the total size of the page becomes O(n**2).
class MDBookSidebarScrollbox extends HTMLElement {
    constructor() {
        super();
    }
    connectedCallback() {
        this.innerHTML = '<ol class="chapter"><li class="chapter-item "><li class="part-title">Introduction</li></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="landing.html"><strong aria-hidden="true">1.</strong> Overview</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="getting-started/index.html"><strong aria-hidden="true">2.</strong> Getting started</a><a class="chapter-fold-toggle"><div>❱</div></a></span><ol class="section"><li class="chapter-item "><span class="chapter-link-wrapper"><a href="getting-started/hardware_requirements.html"><strong aria-hidden="true">2.1.</strong> Hardware requirements</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="getting-started/installation/index.html"><strong aria-hidden="true">2.2.</strong> Installation</a><a class="chapter-fold-toggle"><div>❱</div></a></span><ol class="section"><li class="chapter-item "><span class="chapter-link-wrapper"><a href="getting-started/installation/binary_distribution.html"><strong aria-hidden="true">2.2.1.</strong> Binary distribution</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="getting-started/installation/package_manager.html"><strong aria-hidden="true">2.2.2.</strong> Package manager</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="getting-started/installation/docker_images.html"><strong aria-hidden="true">2.2.3.</strong> Docker image</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="getting-started/installation/building_from_source.html"><strong aria-hidden="true">2.2.4.</strong> Building from source</a></span></li></ol></li></ol><li class="chapter-item "><li class="part-title">Ethrex for Ethereum node operators</li></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l1/running/index.html"><strong aria-hidden="true">3.</strong> Running a node</a><a class="chapter-fold-toggle"><div>❱</div></a></span><ol class="section"><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l1/running/consensus_client.html"><strong aria-hidden="true">3.1.</strong> Connecting to a consensus client</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l1/running/startup.html"><strong aria-hidden="true">3.2.</strong> Node startup</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l1/running/configuration.html"><strong aria-hidden="true">3.3.</strong> Configuration</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l1/running/monitoring.html"><strong aria-hidden="true">3.4.</strong> Monitoring and metrics</a></span></li></ol><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l1/architecture/index.html"><strong aria-hidden="true">4.</strong> Architecture</a><a class="chapter-fold-toggle"><div>❱</div></a></span><ol class="section"><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l1/architecture/overview.html"><strong aria-hidden="true">4.1.</strong> System Overview</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l1/architecture/block_execution.html"><strong aria-hidden="true">4.2.</strong> Block Execution Pipeline</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l1/architecture/sync_state_machine.html"><strong aria-hidden="true">4.3.</strong> Sync State Machine</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l1/architecture/crate_map.html"><strong aria-hidden="true">4.4.</strong> Crate Map</a></span></li></ol><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l1/fundamentals/index.html"><strong aria-hidden="true">5.</strong> Fundamentals</a><a class="chapter-fold-toggle"><div>❱</div></a></span><ol class="section"><li class="chapter-item "><span class="chapter-link-wrapper"><span><strong aria-hidden="true">5.1.</strong> Metrics</span></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><span><strong aria-hidden="true">5.2.</strong> Logs</span></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><span><strong aria-hidden="true">5.3.</strong> Security</span></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l1/fundamentals/databases.html"><strong aria-hidden="true">5.4.</strong> Databases</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l1/fundamentals/networking.html"><strong aria-hidden="true">5.5.</strong> Networking</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l1/fundamentals/sync_modes.html"><strong aria-hidden="true">5.6.</strong> Sync modes</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l1/fundamentals/snap_sync.html"><strong aria-hidden="true">5.7.</strong> Snap sync internals</a><a class="chapter-fold-toggle"><div>❱</div></a></span><ol class="section"><li class="chapter-item "><span class="chapter-link-wrapper"><a href="internal/l1/delete_accounts.html"><strong aria-hidden="true">5.7.1.</strong> Can an account disappear from Ethereum&#39;s state trie?</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l1/fundamentals/snap_sync_concerns.html"><strong aria-hidden="true">5.7.2.</strong> Future Rewrite Concerns</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="internal/l1/sorted_trie_insert.html"><strong aria-hidden="true">5.7.3.</strong> Sorted Trie Insertion</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="internal/l1/healing.html"><strong aria-hidden="true">5.7.4.</strong> Healing Explanation</a></span></li></ol><li class="chapter-item "><span class="chapter-link-wrapper"><span><strong aria-hidden="true">5.8.</strong> Pruning</span></span></li></ol><li class="chapter-item "><li class="part-title">Ethrex for L2 chains</li></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/introduction.html"><strong aria-hidden="true">6.</strong> Introduction</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/deployment/index.html"><strong aria-hidden="true">7.</strong> Deploy an L2</a><a class="chapter-fold-toggle"><div>❱</div></a></span><ol class="section"><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/deployment/overview.html"><strong aria-hidden="true">7.1.</strong> Overview</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/deployment/vanilla.html"><strong aria-hidden="true">7.2.</strong> Deploying a vanilla ethrex L2</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/deployment/validium.html"><strong aria-hidden="true">7.3.</strong> Deploying a validium ethrex L2</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/deployment/based.html"><strong aria-hidden="true">7.4.</strong> Deploying a based ethrex L2</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/deployment/aligned.html"><strong aria-hidden="true">7.5.</strong> Ethrex &lt;&gt; Aligned</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/deployment/aligned_failure_recovery.html"><strong aria-hidden="true">7.6.</strong> Ethrex &lt;&gt; Aligned | Failure Recovery Guide</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/deployment/synchronous_composability_poc.html"><strong aria-hidden="true">7.7.</strong> Synchronous Composability PoC</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/deployment/shared_bridge.html"><strong aria-hidden="true">7.8.</strong> Deploying a shared bridge enabled L2</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/deployment/fee_token.html"><strong aria-hidden="true">7.9.</strong> Deploying a fee token</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/deployment/upgrades.html"><strong aria-hidden="true">7.10.</strong> Upgrades</a></span></li></ol><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/deployment/prover/index.html"><strong aria-hidden="true">8.</strong> Run a prover</a><a class="chapter-fold-toggle"><div>❱</div></a></span><ol class="section"><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/deployment/prover/overview.html"><strong aria-hidden="true">8.1.</strong> Overview</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/deployment/prover/sp1.html"><strong aria-hidden="true">8.2.</strong> Run an ethrex SP1 prover</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/deployment/prover/risc0.html"><strong aria-hidden="true">8.3.</strong> Run an ethrex RISC0 prover</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/deployment/prover/tee.html"><strong aria-hidden="true">8.4.</strong> Run an ethrex TDX prover</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/deployment/prover/multi-prover.html"><strong aria-hidden="true">8.5.</strong> Run multiple provers</a></span></li></ol><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/monitoring.html"><strong aria-hidden="true">9.</strong> Monitoring and metrics</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/admin.html"><strong aria-hidden="true">10.</strong> Admin server</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/stages.html"><strong aria-hidden="true">11.</strong> Ethrex Rollup Stages</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/architecture/index.html"><strong aria-hidden="true">12.</strong> Architecture</a><a class="chapter-fold-toggle"><div>❱</div></a></span><ol class="section"><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/architecture/overview.html"><strong aria-hidden="true">12.1.</strong> Overview</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/architecture/sequencer.html"><strong aria-hidden="true">12.2.</strong> Sequencer</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/architecture/prover.html"><strong aria-hidden="true">12.3.</strong> Prover</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/architecture/tdx.html"><strong aria-hidden="true">12.4.</strong> TDX execution module</a></span></li></ol><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/interacting/index.html"><strong aria-hidden="true">13.</strong> Interacting with the L2</a><a class="chapter-fold-toggle"><div>❱</div></a></span><ol class="section"><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/interacting/deposit.html"><strong aria-hidden="true">13.1.</strong> Deposit assets</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/interacting/withdraw.html"><strong aria-hidden="true">13.2.</strong> Withdraw assets</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/interacting/wallet.html"><strong aria-hidden="true">13.3.</strong> Connect a wallet</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/interacting/deploy_contracts.html"><strong aria-hidden="true">13.4.</strong> Deploy a contract</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/interacting/blockscout.html"><strong aria-hidden="true">13.5.</strong> Blockscout for ethrex L2</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/interacting/l2_hub.html"><strong aria-hidden="true">13.6.</strong> L2 Hub</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/interacting/shared_bridge.html"><strong aria-hidden="true">13.7.</strong> Shared Bridge</a></span></li></ol><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/fundamentals/index.html"><strong aria-hidden="true">14.</strong> Fundamentals</a><a class="chapter-fold-toggle"><div>❱</div></a></span><ol class="section"><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/fundamentals/state_diffs.html"><strong aria-hidden="true">14.1.</strong> State diffs</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/fundamentals/block_vs_state_diff_measurements.html"><strong aria-hidden="true">14.2.</strong> Block vs StateDiff</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/fundamentals/data_availability.html"><strong aria-hidden="true">14.3.</strong> Data availability</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/fundamentals/execution_witness.html"><strong aria-hidden="true">14.4.</strong> Execution witness</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/fundamentals/execution_witness_measurements.html"><strong aria-hidden="true">14.5.</strong> Execution witness measurements</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/fundamentals/deposits.html"><strong aria-hidden="true">14.6.</strong> Deposits</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/fundamentals/withdrawals.html"><strong aria-hidden="true">14.7.</strong> Withdrawals</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/fundamentals/contracts.html"><strong aria-hidden="true">14.8.</strong> Smart contracts</a><a class="chapter-fold-toggle"><div>❱</div></a></span><ol class="section"><li class="chapter-item "><span class="chapter-link-wrapper"><span><strong aria-hidden="true">14.8.1.</strong> OnChainOperator</span></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><span><strong aria-hidden="true">14.8.2.</strong> CommonBridge</span></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><span><strong aria-hidden="true">14.8.3.</strong> L1MessageSender</span></span></li></ol><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/fundamentals/based.html"><strong aria-hidden="true">14.9.</strong> Based sequencing</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/fundamentals/transaction_fees.html"><strong aria-hidden="true">14.10.</strong> Transaction fees</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/fundamentals/exit_window.html"><strong aria-hidden="true">14.11.</strong> Exit Window</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/fundamentals/timelock.html"><strong aria-hidden="true">14.12.</strong> Timelock</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/fundamentals/fee_token.html"><strong aria-hidden="true">14.13.</strong> Fee token</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/fundamentals/shared_bridge.html"><strong aria-hidden="true">14.14.</strong> Shared Bridge</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/fundamentals/ethrex_l2_aligned_integration.html"><strong aria-hidden="true">14.15.</strong> Aligned Layer Integration</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/fundamentals/distributed_proving.html"><strong aria-hidden="true">14.16.</strong> Distributed proving</a></span></li></ol><li class="chapter-item "><li class="part-title">Ethrex for developers</li></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/index.html"><strong aria-hidden="true">15.</strong> Getting started</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/installing.html"><strong aria-hidden="true">16.</strong> Building</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/l1/introduction.html"><strong aria-hidden="true">17.</strong> L1</a><a class="chapter-fold-toggle"><div>❱</div></a></span><ol class="section"><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/l1/dev-mode.html"><strong aria-hidden="true">17.1.</strong> Ethrex as a local development node</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/l1/importing-blocks.html"><strong aria-hidden="true">17.2.</strong> Importing blocks from a file</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/l1/kurtosis-localnet.html"><strong aria-hidden="true">17.3.</strong> Kurtosis localnet</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/l1/metrics.html"><strong aria-hidden="true">17.4.</strong> Metrics</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/l1/dashboards.html"><strong aria-hidden="true">17.5.</strong> Dashboards</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/l1/profiling.html"><strong aria-hidden="true">17.6.</strong> Profiling</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/l1/storage_api.html"><strong aria-hidden="true">17.7.</strong> Integrating with other DBs</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/l1/testing/index.html"><strong aria-hidden="true">17.8.</strong> Testing</a><a class="chapter-fold-toggle"><div>❱</div></a></span><ol class="section"><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/l1/testing/ef-tests.html"><strong aria-hidden="true">17.8.1.</strong> Ethereum foundation tests</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/l1/testing/hive.html"><strong aria-hidden="true">17.8.2.</strong> Hive tests</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/l1/testing/assertoor.html"><strong aria-hidden="true">17.8.3.</strong> Assertoor tests</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/l1/testing/rust.html"><strong aria-hidden="true">17.8.4.</strong> Rust tests</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/l1/testing/load-tests.html"><strong aria-hidden="true">17.8.5.</strong> Load tests</a></span></li></ol></li></ol><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/l2/introduction.html"><strong aria-hidden="true">18.</strong> L2</a><a class="chapter-fold-toggle"><div>❱</div></a></span><ol class="section"><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/l2/dev-mode.html"><strong aria-hidden="true">18.1.</strong> Ethrex L2 as local development mode</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/l2/integration-tests.html"><strong aria-hidden="true">18.2.</strong> Integration tests</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/l2/prover.html"><strong aria-hidden="true">18.3.</strong> Running the Prover</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/l2/state-reconstruction-blobs.html"><strong aria-hidden="true">18.4.</strong> Generate blobs for the state reconstruction test</a></span></li></ol><li class="chapter-item "><span class="chapter-link-wrapper"><a href="prover/prover.html"><strong aria-hidden="true">19.</strong> Prover</a><a class="chapter-fold-toggle"><div>❱</div></a></span><ol class="section"><li class="chapter-item "><span class="chapter-link-wrapper"><a href="prover/guest_program.html"><strong aria-hidden="true">19.1.</strong> Guest program</a></span></li></ol><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/rich-accounts.html"><strong aria-hidden="true">20.</strong> Rich Accounts</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="vm/levm/debug.html"><strong aria-hidden="true">21.</strong> Debugging solidity with ethrex</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="ethrex_replay/ethrex_replay.html"><strong aria-hidden="true">22.</strong> Re-execute Ethereum with ethrex</a><a class="chapter-fold-toggle"><div>❱</div></a></span><ol class="section"><li class="chapter-item "><span class="chapter-link-wrapper"><a href="ethrex_replay/faq.html"><strong aria-hidden="true">22.1.</strong> FAQ</a></span></li></ol><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/repl.html"><strong aria-hidden="true">23.</strong> Interactive REPL</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="CLI.html"><strong aria-hidden="true">24.</strong> CLI reference</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="developers/release-process.html"><strong aria-hidden="true">25.</strong> Release Process</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><span><strong aria-hidden="true">26.</strong> Troubleshooting</span></span></li><li class="chapter-item "><li class="part-title">Roadmap</li></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="roadmap.html"><strong aria-hidden="true">27.</strong> Roadmap</a></span></li><li class="chapter-item "><li class="part-title">Why ethrex</li></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="zkvm-integrations.html"><strong aria-hidden="true">28.</strong> zkVM Integrations</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/bench/zkvm_comparison.html"><strong aria-hidden="true">29.</strong> Benchmark Comparisons</a><a class="chapter-fold-toggle"><div>❱</div></a></span><ol class="section"><li class="chapter-item "><span class="chapter-link-wrapper"><a href="l2/bench/prover_performance.html"><strong aria-hidden="true">29.1.</strong> Prover Performance</a></span></li></ol><li class="chapter-item "><span class="chapter-link-wrapper"><a href="case-studies.html"><strong aria-hidden="true">30.</strong> Case Studies</a></span></li><li class="chapter-item "><li class="part-title">Other resources</li></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="audits/index.html"><strong aria-hidden="true">31.</strong> Audits</a><a class="chapter-fold-toggle"><div>❱</div></a></span><ol class="section"><li class="chapter-item "><span class="chapter-link-wrapper"><a href="audits/least-authority-initial-audit.html"><strong aria-hidden="true">31.1.</strong> Least Authority Initial Audit (v9.0.0)</a></span></li></ol><li class="chapter-item "><span class="chapter-link-wrapper"><a href="CONTRIBUTING_DOCS.html"><strong aria-hidden="true">32.</strong> Contributing to the Documentation</a></span></li><li class="chapter-item "><span class="chapter-link-wrapper"><a href="lectures.html"><strong aria-hidden="true">33.</strong> Recommended lectures</a></span></li></ol>';
        // Set the current, active page, and reveal it if it's hidden
        let current_page = document.location.href.toString().split('#')[0].split('?')[0];
        if (current_page.endsWith('/')) {
            current_page += 'index.html';
        }
        const links = Array.prototype.slice.call(this.querySelectorAll('a'));
        const l = links.length;
        for (let i = 0; i < l; ++i) {
            const link = links[i];
            const href = link.getAttribute('href');
            if (href && !href.startsWith('#') && !/^(?:[a-z+]+:)?\/\//.test(href)) {
                link.href = path_to_root + href;
            }
            // The 'index' page is supposed to alias the first chapter in the book.
            if (link.href === current_page
                || i === 0
                && path_to_root === ''
                && current_page.endsWith('/index.html')) {
                link.classList.add('active');
                let parent = link.parentElement;
                while (parent) {
                    if (parent.tagName === 'LI' && parent.classList.contains('chapter-item')) {
                        parent.classList.add('expanded');
                    }
                    parent = parent.parentElement;
                }
            }
        }
        // Track and set sidebar scroll position
        this.addEventListener('click', e => {
            if (e.target.tagName === 'A') {
                const clientRect = e.target.getBoundingClientRect();
                const sidebarRect = this.getBoundingClientRect();
                sessionStorage.setItem('sidebar-scroll-offset', clientRect.top - sidebarRect.top);
            }
        }, { passive: true });
        const sidebarScrollOffset = sessionStorage.getItem('sidebar-scroll-offset');
        sessionStorage.removeItem('sidebar-scroll-offset');
        if (sidebarScrollOffset !== null) {
            // preserve sidebar scroll position when navigating via links within sidebar
            const activeSection = this.querySelector('.active');
            if (activeSection) {
                const clientRect = activeSection.getBoundingClientRect();
                const sidebarRect = this.getBoundingClientRect();
                const currentOffset = clientRect.top - sidebarRect.top;
                this.scrollTop += currentOffset - parseFloat(sidebarScrollOffset);
            }
        } else {
            // scroll sidebar to current active section when navigating via
            // 'next/previous chapter' buttons
            const activeSection = document.querySelector('#mdbook-sidebar .active');
            if (activeSection) {
                activeSection.scrollIntoView({ block: 'center' });
            }
        }
        // Toggle buttons
        const sidebarAnchorToggles = document.querySelectorAll('.chapter-fold-toggle');
        function toggleSection(ev) {
            ev.currentTarget.parentElement.parentElement.classList.toggle('expanded');
        }
        Array.from(sidebarAnchorToggles).forEach(el => {
            el.addEventListener('click', toggleSection);
        });
    }
}
window.customElements.define('mdbook-sidebar-scrollbox', MDBookSidebarScrollbox);


// ---------------------------------------------------------------------------
// Support for dynamically adding headers to the sidebar.

(function() {
    // This is used to detect which direction the page has scrolled since the
    // last scroll event.
    let lastKnownScrollPosition = 0;
    // This is the threshold in px from the top of the screen where it will
    // consider a header the "current" header when scrolling down.
    const defaultDownThreshold = 150;
    // Same as defaultDownThreshold, except when scrolling up.
    const defaultUpThreshold = 300;
    // The threshold is a virtual horizontal line on the screen where it
    // considers the "current" header to be above the line. The threshold is
    // modified dynamically to handle headers that are near the bottom of the
    // screen, and to slightly offset the behavior when scrolling up vs down.
    let threshold = defaultDownThreshold;
    // This is used to disable updates while scrolling. This is needed when
    // clicking the header in the sidebar, which triggers a scroll event. It
    // is somewhat finicky to detect when the scroll has finished, so this
    // uses a relatively dumb system of disabling scroll updates for a short
    // time after the click.
    let disableScroll = false;
    // Array of header elements on the page.
    let headers;
    // Array of li elements that are initially collapsed headers in the sidebar.
    // I'm not sure why eslint seems to have a false positive here.
    // eslint-disable-next-line prefer-const
    let headerToggles = [];
    // This is a debugging tool for the threshold which you can enable in the console.
    let thresholdDebug = false;

    // Updates the threshold based on the scroll position.
    function updateThreshold() {
        const scrollTop = window.pageYOffset || document.documentElement.scrollTop;
        const windowHeight = window.innerHeight;
        const documentHeight = document.documentElement.scrollHeight;

        // The number of pixels below the viewport, at most documentHeight.
        // This is used to push the threshold down to the bottom of the page
        // as the user scrolls towards the bottom.
        const pixelsBelow = Math.max(0, documentHeight - (scrollTop + windowHeight));
        // The number of pixels above the viewport, at least defaultDownThreshold.
        // Similar to pixelsBelow, this is used to push the threshold back towards
        // the top when reaching the top of the page.
        const pixelsAbove = Math.max(0, defaultDownThreshold - scrollTop);
        // How much the threshold should be offset once it gets close to the
        // bottom of the page.
        const bottomAdd = Math.max(0, windowHeight - pixelsBelow - defaultDownThreshold);
        let adjustedBottomAdd = bottomAdd;

        // Adjusts bottomAdd for a small document. The calculation above
        // assumes the document is at least twice the windowheight in size. If
        // it is less than that, then bottomAdd needs to be shrunk
        // proportional to the difference in size.
        if (documentHeight < windowHeight * 2) {
            const maxPixelsBelow = documentHeight - windowHeight;
            const t = 1 - pixelsBelow / Math.max(1, maxPixelsBelow);
            const clamp = Math.max(0, Math.min(1, t));
            adjustedBottomAdd *= clamp;
        }

        let scrollingDown = true;
        if (scrollTop < lastKnownScrollPosition) {
            scrollingDown = false;
        }

        if (scrollingDown) {
            // When scrolling down, move the threshold up towards the default
            // downwards threshold position. If near the bottom of the page,
            // adjustedBottomAdd will offset the threshold towards the bottom
            // of the page.
            const amountScrolledDown = scrollTop - lastKnownScrollPosition;
            const adjustedDefault = defaultDownThreshold + adjustedBottomAdd;
            threshold = Math.max(adjustedDefault, threshold - amountScrolledDown);
        } else {
            // When scrolling up, move the threshold down towards the default
            // upwards threshold position. If near the bottom of the page,
            // quickly transition the threshold back up where it normally
            // belongs.
            const amountScrolledUp = lastKnownScrollPosition - scrollTop;
            const adjustedDefault = defaultUpThreshold - pixelsAbove
                + Math.max(0, adjustedBottomAdd - defaultDownThreshold);
            threshold = Math.min(adjustedDefault, threshold + amountScrolledUp);
        }

        if (documentHeight <= windowHeight) {
            threshold = 0;
        }

        if (thresholdDebug) {
            const id = 'mdbook-threshold-debug-data';
            let data = document.getElementById(id);
            if (data === null) {
                data = document.createElement('div');
                data.id = id;
                data.style.cssText = `
                    position: fixed;
                    top: 50px;
                    right: 10px;
                    background-color: 0xeeeeee;
                    z-index: 9999;
                    pointer-events: none;
                `;
                document.body.appendChild(data);
            }
            data.innerHTML = `
                <table>
                  <tr><td>documentHeight</td><td>${documentHeight.toFixed(1)}</td></tr>
                  <tr><td>windowHeight</td><td>${windowHeight.toFixed(1)}</td></tr>
                  <tr><td>scrollTop</td><td>${scrollTop.toFixed(1)}</td></tr>
                  <tr><td>pixelsAbove</td><td>${pixelsAbove.toFixed(1)}</td></tr>
                  <tr><td>pixelsBelow</td><td>${pixelsBelow.toFixed(1)}</td></tr>
                  <tr><td>bottomAdd</td><td>${bottomAdd.toFixed(1)}</td></tr>
                  <tr><td>adjustedBottomAdd</td><td>${adjustedBottomAdd.toFixed(1)}</td></tr>
                  <tr><td>scrollingDown</td><td>${scrollingDown}</td></tr>
                  <tr><td>threshold</td><td>${threshold.toFixed(1)}</td></tr>
                </table>
            `;
            drawDebugLine();
        }

        lastKnownScrollPosition = scrollTop;
    }

    function drawDebugLine() {
        if (!document.body) {
            return;
        }
        const id = 'mdbook-threshold-debug-line';
        const existingLine = document.getElementById(id);
        if (existingLine) {
            existingLine.remove();
        }
        const line = document.createElement('div');
        line.id = id;
        line.style.cssText = `
            position: fixed;
            top: ${threshold}px;
            left: 0;
            width: 100vw;
            height: 2px;
            background-color: red;
            z-index: 9999;
            pointer-events: none;
        `;
        document.body.appendChild(line);
    }

    function mdbookEnableThresholdDebug() {
        thresholdDebug = true;
        updateThreshold();
        drawDebugLine();
    }

    window.mdbookEnableThresholdDebug = mdbookEnableThresholdDebug;

    // Updates which headers in the sidebar should be expanded. If the current
    // header is inside a collapsed group, then it, and all its parents should
    // be expanded.
    function updateHeaderExpanded(currentA) {
        // Add expanded to all header-item li ancestors.
        let current = currentA.parentElement;
        while (current) {
            if (current.tagName === 'LI' && current.classList.contains('header-item')) {
                current.classList.add('expanded');
            }
            current = current.parentElement;
        }
    }

    // Updates which header is marked as the "current" header in the sidebar.
    // This is done with a virtual Y threshold, where headers at or below
    // that line will be considered the current one.
    function updateCurrentHeader() {
        if (!headers || !headers.length) {
            return;
        }

        // Reset the classes, which will be rebuilt below.
        const els = document.getElementsByClassName('current-header');
        for (const el of els) {
            el.classList.remove('current-header');
        }
        for (const toggle of headerToggles) {
            toggle.classList.remove('expanded');
        }

        // Find the last header that is above the threshold.
        let lastHeader = null;
        for (const header of headers) {
            const rect = header.getBoundingClientRect();
            if (rect.top <= threshold) {
                lastHeader = header;
            } else {
                break;
            }
        }
        if (lastHeader === null) {
            lastHeader = headers[0];
            const rect = lastHeader.getBoundingClientRect();
            const windowHeight = window.innerHeight;
            if (rect.top >= windowHeight) {
                return;
            }
        }

        // Get the anchor in the summary.
        const href = '#' + lastHeader.id;
        const a = [...document.querySelectorAll('.header-in-summary')]
            .find(element => element.getAttribute('href') === href);
        if (!a) {
            return;
        }

        a.classList.add('current-header');

        updateHeaderExpanded(a);
    }

    // Updates which header is "current" based on the threshold line.
    function reloadCurrentHeader() {
        if (disableScroll) {
            return;
        }
        updateThreshold();
        updateCurrentHeader();
    }


    // When clicking on a header in the sidebar, this adjusts the threshold so
    // that it is located next to the header. This is so that header becomes
    // "current".
    function headerThresholdClick(event) {
        // See disableScroll description why this is done.
        disableScroll = true;
        setTimeout(() => {
            disableScroll = false;
        }, 100);
        // requestAnimationFrame is used to delay the update of the "current"
        // header until after the scroll is done, and the header is in the new
        // position.
        requestAnimationFrame(() => {
            requestAnimationFrame(() => {
                // Closest is needed because if it has child elements like <code>.
                const a = event.target.closest('a');
                const href = a.getAttribute('href');
                const targetId = href.substring(1);
                const targetElement = document.getElementById(targetId);
                if (targetElement) {
                    threshold = targetElement.getBoundingClientRect().bottom;
                    updateCurrentHeader();
                }
            });
        });
    }

    // Takes the nodes from the given head and copies them over to the
    // destination, along with some filtering.
    function filterHeader(source, dest) {
        const clone = source.cloneNode(true);
        clone.querySelectorAll('mark').forEach(mark => {
            mark.replaceWith(...mark.childNodes);
        });
        dest.append(...clone.childNodes);
    }

    // Scans page for headers and adds them to the sidebar.
    document.addEventListener('DOMContentLoaded', function() {
        const activeSection = document.querySelector('#mdbook-sidebar .active');
        if (activeSection === null) {
            return;
        }

        const main = document.getElementsByTagName('main')[0];
        headers = Array.from(main.querySelectorAll('h2, h3, h4, h5, h6'))
            .filter(h => h.id !== '' && h.children.length && h.children[0].tagName === 'A');

        if (headers.length === 0) {
            return;
        }

        // Build a tree of headers in the sidebar.

        const stack = [];

        const firstLevel = parseInt(headers[0].tagName.charAt(1));
        for (let i = 1; i < firstLevel; i++) {
            const ol = document.createElement('ol');
            ol.classList.add('section');
            if (stack.length > 0) {
                stack[stack.length - 1].ol.appendChild(ol);
            }
            stack.push({level: i + 1, ol: ol});
        }

        // The level where it will start folding deeply nested headers.
        const foldLevel = 3;

        for (let i = 0; i < headers.length; i++) {
            const header = headers[i];
            const level = parseInt(header.tagName.charAt(1));

            const currentLevel = stack[stack.length - 1].level;
            if (level > currentLevel) {
                // Begin nesting to this level.
                for (let nextLevel = currentLevel + 1; nextLevel <= level; nextLevel++) {
                    const ol = document.createElement('ol');
                    ol.classList.add('section');
                    const last = stack[stack.length - 1];
                    const lastChild = last.ol.lastChild;
                    // Handle the case where jumping more than one nesting
                    // level, which doesn't have a list item to place this new
                    // list inside of.
                    if (lastChild) {
                        lastChild.appendChild(ol);
                    } else {
                        last.ol.appendChild(ol);
                    }
                    stack.push({level: nextLevel, ol: ol});
                }
            } else if (level < currentLevel) {
                while (stack.length > 1 && stack[stack.length - 1].level > level) {
                    stack.pop();
                }
            }

            const li = document.createElement('li');
            li.classList.add('header-item');
            li.classList.add('expanded');
            if (level < foldLevel) {
                li.classList.add('expanded');
            }
            const span = document.createElement('span');
            span.classList.add('chapter-link-wrapper');
            const a = document.createElement('a');
            span.appendChild(a);
            a.href = '#' + header.id;
            a.classList.add('header-in-summary');
            filterHeader(header.children[0], a);
            a.addEventListener('click', headerThresholdClick);
            const nextHeader = headers[i + 1];
            if (nextHeader !== undefined) {
                const nextLevel = parseInt(nextHeader.tagName.charAt(1));
                if (nextLevel > level && level >= foldLevel) {
                    const toggle = document.createElement('a');
                    toggle.classList.add('chapter-fold-toggle');
                    toggle.classList.add('header-toggle');
                    toggle.addEventListener('click', () => {
                        li.classList.toggle('expanded');
                    });
                    const toggleDiv = document.createElement('div');
                    toggleDiv.textContent = '❱';
                    toggle.appendChild(toggleDiv);
                    span.appendChild(toggle);
                    headerToggles.push(li);
                }
            }
            li.appendChild(span);

            const currentParent = stack[stack.length - 1];
            currentParent.ol.appendChild(li);
        }

        const onThisPage = document.createElement('div');
        onThisPage.classList.add('on-this-page');
        onThisPage.append(stack[0].ol);
        const activeItemSpan = activeSection.parentElement;
        activeItemSpan.after(onThisPage);
    });

    document.addEventListener('DOMContentLoaded', reloadCurrentHeader);
    document.addEventListener('scroll', reloadCurrentHeader, { passive: true });
})();

