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
        this.innerHTML = '<ol class="chapter"><li class="chapter-item affix "><li class="part-title">Introduction</li><li class="chapter-item "><a href="getting-started/index.html"><strong aria-hidden="true">1.</strong> Getting started</a><a class="toggle"><div>❱</div></a></li><li><ol class="section"><li class="chapter-item "><div><strong aria-hidden="true">1.1.</strong> Hardware requirements</div></li><li class="chapter-item "><a href="getting-started/installation/index.html"><strong aria-hidden="true">1.2.</strong> Installation</a><a class="toggle"><div>❱</div></a></li><li><ol class="section"><li class="chapter-item "><a href="getting-started/installation/binary_distribution.html"><strong aria-hidden="true">1.2.1.</strong> Binary distribution</a></li><li class="chapter-item "><a href="getting-started/installation/package_manager.html"><strong aria-hidden="true">1.2.2.</strong> Package manager</a></li><li class="chapter-item "><a href="getting-started/installation/docker_images.html"><strong aria-hidden="true">1.2.3.</strong> Docker image</a></li><li class="chapter-item "><a href="getting-started/installation/building_from_source.html"><strong aria-hidden="true">1.2.4.</strong> Building from source</a></li></ol></li></ol></li><li class="chapter-item "><li class="part-title">Ethrex for Ethereum node operators</li><li class="chapter-item "><a href="l1/running/index.html"><strong aria-hidden="true">2.</strong> Running a node</a><a class="toggle"><div>❱</div></a></li><li><ol class="section"><li class="chapter-item "><a href="l1/running/consensus_client.html"><strong aria-hidden="true">2.1.</strong> Connecting to a consensus client</a></li><li class="chapter-item "><a href="l1/running/startup.html"><strong aria-hidden="true">2.2.</strong> Node startup</a></li><li class="chapter-item "><a href="l1/running/configuration.html"><strong aria-hidden="true">2.3.</strong> Configuration</a></li><li class="chapter-item "><a href="l1/running/monitoring.html"><strong aria-hidden="true">2.4.</strong> Monitoring and metrics</a></li></ol></li><li class="chapter-item "><a href="l1/fundamentals/index.html"><strong aria-hidden="true">3.</strong> Fundamentals</a><a class="toggle"><div>❱</div></a></li><li><ol class="section"><li class="chapter-item "><div><strong aria-hidden="true">3.1.</strong> Metrics</div></li><li class="chapter-item "><div><strong aria-hidden="true">3.2.</strong> Logs</div></li><li class="chapter-item "><div><strong aria-hidden="true">3.3.</strong> Security</div></li><li class="chapter-item "><div><strong aria-hidden="true">3.4.</strong> Databases</div></li><li class="chapter-item "><a href="l1/fundamentals/networking.html"><strong aria-hidden="true">3.5.</strong> Networking</a></li><li class="chapter-item "><a href="l1/fundamentals/sync_modes.html"><strong aria-hidden="true">3.6.</strong> Sync modes</a></li><li class="chapter-item "><div><strong aria-hidden="true">3.7.</strong> Pruning</div></li></ol></li><li class="chapter-item "><li class="part-title">Ethrex for L2 chains</li><li class="chapter-item "><a href="l2/introduction.html"><strong aria-hidden="true">4.</strong> Introduction</a></li><li class="chapter-item "><a href="l2/deploy.html"><strong aria-hidden="true">5.</strong> Deploy an L2</a></li><li class="chapter-item "><a href="l2/monitoring.html"><strong aria-hidden="true">6.</strong> Monitoring and metrics</a></li><li class="chapter-item "><a href="l2/admin.html"><strong aria-hidden="true">7.</strong> Admin server</a></li><li class="chapter-item "><a href="l2/architecture/index.html"><strong aria-hidden="true">8.</strong> Architecture</a><a class="toggle"><div>❱</div></a></li><li><ol class="section"><li class="chapter-item "><a href="l2/architecture/overview.html"><strong aria-hidden="true">8.1.</strong> Overview</a></li><li class="chapter-item "><a href="l2/architecture/sequencer.html"><strong aria-hidden="true">8.2.</strong> Sequencer</a></li><li class="chapter-item "><a href="l2/architecture/prover.html"><strong aria-hidden="true">8.3.</strong> Prover</a></li><li class="chapter-item "><a href="l2/architecture/aligned_mode.html"><strong aria-hidden="true">8.4.</strong> Aligned mode</a></li><li class="chapter-item "><a href="l2/architecture/tdx.html"><strong aria-hidden="true">8.5.</strong> TDX execution module</a></li></ol></li><li class="chapter-item "><a href="l2/interacting/index.html"><strong aria-hidden="true">9.</strong> Interacting with the L2</a><a class="toggle"><div>❱</div></a></li><li><ol class="section"><li class="chapter-item "><a href="l2/interacting/deposit.html"><strong aria-hidden="true">9.1.</strong> Deposit assets</a></li><li class="chapter-item "><a href="l2/interacting/withdraw.html"><strong aria-hidden="true">9.2.</strong> Withdraw assets</a></li><li class="chapter-item "><a href="l2/interacting/wallet.html"><strong aria-hidden="true">9.3.</strong> Connect a wallet</a></li><li class="chapter-item "><a href="l2/interacting/deploy_contracts.html"><strong aria-hidden="true">9.4.</strong> Deploy a contract</a></li></ol></li><li class="chapter-item "><a href="l2/fundamentals/index.html"><strong aria-hidden="true">10.</strong> Fundamentals</a><a class="toggle"><div>❱</div></a></li><li><ol class="section"><li class="chapter-item "><a href="l2/fundamentals/state_diffs.html"><strong aria-hidden="true">10.1.</strong> State diffs</a></li><li class="chapter-item "><a href="l2/fundamentals/deposits.html"><strong aria-hidden="true">10.2.</strong> Deposits</a></li><li class="chapter-item "><a href="l2/fundamentals/withdrawals.html"><strong aria-hidden="true">10.3.</strong> Withdrawals</a></li><li class="chapter-item "><a href="l2/fundamentals/contracts.html"><strong aria-hidden="true">10.4.</strong> Smart contracts</a><a class="toggle"><div>❱</div></a></li><li><ol class="section"><li class="chapter-item "><div><strong aria-hidden="true">10.4.1.</strong> OnChainOperator</div></li><li class="chapter-item "><div><strong aria-hidden="true">10.4.2.</strong> CommonBridge</div></li><li class="chapter-item "><div><strong aria-hidden="true">10.4.3.</strong> L1MessageSender</div></li></ol></li><li class="chapter-item "><a href="l2/fundamentals/based.html"><strong aria-hidden="true">10.5.</strong> Based sequencing</a></li><li class="chapter-item "><a href="l2/fundamentals/transaction_fees.html"><strong aria-hidden="true">10.6.</strong> Transaction fees</a></li></ol></li><li class="chapter-item "><li class="part-title">Ethrex for developers</li><li class="chapter-item "><a href="developers/index.html"><strong aria-hidden="true">11.</strong> Getting started</a></li><li class="chapter-item "><a href="developers/installing.html"><strong aria-hidden="true">12.</strong> Building</a></li><li class="chapter-item "><a href="developers/l1/introduction.html"><strong aria-hidden="true">13.</strong> L1</a><a class="toggle"><div>❱</div></a></li><li><ol class="section"><li class="chapter-item "><a href="developers/l1/dev-mode.html"><strong aria-hidden="true">13.1.</strong> Ethrex as a local development node</a></li><li class="chapter-item "><a href="developers/l1/importing-blocks.html"><strong aria-hidden="true">13.2.</strong> Importing blocks from a file</a></li><li class="chapter-item "><a href="developers/l1/kurtosis-localnet.html"><strong aria-hidden="true">13.3.</strong> Kurtosis localnet</a></li><li class="chapter-item "><a href="developers/l1/metrics.html"><strong aria-hidden="true">13.4.</strong> Metrics</a></li><li class="chapter-item "><a href="developers/l1/testing/index.html"><strong aria-hidden="true">13.5.</strong> Testing</a><a class="toggle"><div>❱</div></a></li><li><ol class="section"><li class="chapter-item "><a href="developers/l1/testing/ef-tests.html"><strong aria-hidden="true">13.5.1.</strong> Ethereum foundation tests</a></li><li class="chapter-item "><a href="developers/l1/testing/hive.html"><strong aria-hidden="true">13.5.2.</strong> Hive tests</a></li><li class="chapter-item "><a href="developers/l1/testing/assertoor.html"><strong aria-hidden="true">13.5.3.</strong> Assertoor tests</a></li><li class="chapter-item "><a href="developers/l1/testing/rust.html"><strong aria-hidden="true">13.5.4.</strong> Rust tests</a></li><li class="chapter-item "><a href="developers/l1/testing/load-tests.html"><strong aria-hidden="true">13.5.5.</strong> Load tests</a></li></ol></li></ol></li><li class="chapter-item "><a href="developers/l2/introduction.html"><strong aria-hidden="true">14.</strong> L2</a><a class="toggle"><div>❱</div></a></li><li><ol class="section"><li class="chapter-item "><a href="developers/l2/dev-mode.html"><strong aria-hidden="true">14.1.</strong> Ethrex L2 as local development mode</a></li></ol></li><li class="chapter-item "><a href="vm/levm/debug.html"><strong aria-hidden="true">15.</strong> Debugging solidity with ethrex</a></li><li class="chapter-item "><a href="ethrex_replay/ethrex_replay.html"><strong aria-hidden="true">16.</strong> Re-execute Ethereum with ethrex</a><a class="toggle"><div>❱</div></a></li><li><ol class="section"><li class="chapter-item "><a href="ethrex_replay/faq.html"><strong aria-hidden="true">16.1.</strong> FAQ</a></li></ol></li><li class="chapter-item "><a href="CLI.html"><strong aria-hidden="true">17.</strong> CLI reference</a></li><li class="chapter-item "><a href="developers/release-process.html"><strong aria-hidden="true">18.</strong> Release Process</a></li><li class="chapter-item "><div><strong aria-hidden="true">19.</strong> Troubleshooting</div></li><li class="chapter-item affix "><li class="part-title">Roadmap</li><li class="chapter-item "><a href="roadmap.html"><strong aria-hidden="true">20.</strong> Roadmap</a></li><li class="chapter-item affix "><li class="part-title">Other resources</li><li class="chapter-item "><a href="CONTRIBUTING_DOCS.html"><strong aria-hidden="true">21.</strong> Contributing to the Documentation</a></li></ol>';
        // Set the current, active page, and reveal it if it's hidden
        let current_page = document.location.href.toString().split("#")[0].split("?")[0];
        if (current_page.endsWith("/")) {
            current_page += "index.html";
        }
        var links = Array.prototype.slice.call(this.querySelectorAll("a"));
        var l = links.length;
        for (var i = 0; i < l; ++i) {
            var link = links[i];
            var href = link.getAttribute("href");
            if (href && !href.startsWith("#") && !/^(?:[a-z+]+:)?\/\//.test(href)) {
                link.href = path_to_root + href;
            }
            // The "index" page is supposed to alias the first chapter in the book.
            if (link.href === current_page || (i === 0 && path_to_root === "" && current_page.endsWith("/index.html"))) {
                link.classList.add("active");
                var parent = link.parentElement;
                if (parent && parent.classList.contains("chapter-item")) {
                    parent.classList.add("expanded");
                }
                while (parent) {
                    if (parent.tagName === "LI" && parent.previousElementSibling) {
                        if (parent.previousElementSibling.classList.contains("chapter-item")) {
                            parent.previousElementSibling.classList.add("expanded");
                        }
                    }
                    parent = parent.parentElement;
                }
            }
        }
        // Track and set sidebar scroll position
        this.addEventListener('click', function(e) {
            if (e.target.tagName === 'A') {
                sessionStorage.setItem('sidebar-scroll', this.scrollTop);
            }
        }, { passive: true });
        var sidebarScrollTop = sessionStorage.getItem('sidebar-scroll');
        sessionStorage.removeItem('sidebar-scroll');
        if (sidebarScrollTop) {
            // preserve sidebar scroll position when navigating via links within sidebar
            this.scrollTop = sidebarScrollTop;
        } else {
            // scroll sidebar to current active section when navigating via "next/previous chapter" buttons
            var activeSection = document.querySelector('#sidebar .active');
            if (activeSection) {
                activeSection.scrollIntoView({ block: 'center' });
            }
        }
        // Toggle buttons
        var sidebarAnchorToggles = document.querySelectorAll('#sidebar a.toggle');
        function toggleSection(ev) {
            ev.currentTarget.parentElement.classList.toggle('expanded');
        }
        Array.from(sidebarAnchorToggles).forEach(function (el) {
            el.addEventListener('click', toggleSection);
        });
    }
}
window.customElements.define("mdbook-sidebar-scrollbox", MDBookSidebarScrollbox);
