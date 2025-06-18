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
        this.innerHTML = '<ol class="chapter"><li class="chapter-item "><div><strong aria-hidden="true">1.</strong> Getting started</div><a class="toggle"><div>❱</div></a></li><li><ol class="section"><li class="chapter-item "><div><strong aria-hidden="true">1.1.</strong> Installing ethrex</div><a class="toggle"><div>❱</div></a></li><li><ol class="section"><li class="chapter-item "><div><strong aria-hidden="true">1.1.1.</strong> Build from source</div></li></ol></li><li class="chapter-item "><div><strong aria-hidden="true">1.2.</strong> Running a node</div></li><li class="chapter-item "><div><strong aria-hidden="true">1.3.</strong> Migrating from another client</div></li></ol></li><li class="chapter-item "><div><strong aria-hidden="true">2.</strong> Fundamentals</div><a class="toggle"><div>❱</div></a></li><li><ol class="section"><li class="chapter-item "><div><strong aria-hidden="true">2.1.</strong> Metrics</div></li><li class="chapter-item "><div><strong aria-hidden="true">2.2.</strong> Logs</div></li><li class="chapter-item "><div><strong aria-hidden="true">2.3.</strong> Security</div></li><li class="chapter-item "><div><strong aria-hidden="true">2.4.</strong> Databases</div></li><li class="chapter-item "><div><strong aria-hidden="true">2.5.</strong> Sync modes</div></li><li class="chapter-item "><div><strong aria-hidden="true">2.6.</strong> Pruning</div></li></ol></li><li class="chapter-item "><a href="l2/index.html"><strong aria-hidden="true">3.</strong> Layer 2</a><a class="toggle"><div>❱</div></a></li><li><ol class="section"><li class="chapter-item "><a href="l2/getting_started.html"><strong aria-hidden="true">3.1.</strong> Getting started</a></li><li class="chapter-item "><a href="l2/guides/index.html"><strong aria-hidden="true">3.2.</strong> Guides</a><a class="toggle"><div>❱</div></a></li><li><ol class="section"><li class="chapter-item "><a href="l2/guides/depositing.html"><strong aria-hidden="true">3.2.1.</strong> Depositing assets</a></li><li class="chapter-item "><a href="l2/guides/withdrawing.html"><strong aria-hidden="true">3.2.2.</strong> Withdrawing assets</a></li></ol></li><li class="chapter-item "><a href="l2/overview.html"><strong aria-hidden="true">3.3.</strong> General overview</a><a class="toggle"><div>❱</div></a></li><li><ol class="section"><li class="chapter-item "><a href="l2/state_diffs.html"><strong aria-hidden="true">3.3.1.</strong> State diffs</a></li><li class="chapter-item "><a href="l2/withdrawals.html"><strong aria-hidden="true">3.3.2.</strong> Withdrawals</a></li></ol></li><li class="chapter-item "><a href="l2/contracts.html"><strong aria-hidden="true">3.4.</strong> Smart contracts</a><a class="toggle"><div>❱</div></a></li><li><ol class="section"><li class="chapter-item "><div><strong aria-hidden="true">3.4.1.</strong> OnChainOperator</div></li><li class="chapter-item "><div><strong aria-hidden="true">3.4.2.</strong> CommonBridge</div></li><li class="chapter-item "><div><strong aria-hidden="true">3.4.3.</strong> L1MessageSender</div></li></ol></li><li class="chapter-item "><a href="l2/components.html"><strong aria-hidden="true">3.5.</strong> Components</a><a class="toggle"><div>❱</div></a></li><li><ol class="section"><li class="chapter-item "><a href="l2/sequencer.html"><strong aria-hidden="true">3.5.1.</strong> Sequencer</a></li><li class="chapter-item "><a href="l2/prover.html"><strong aria-hidden="true">3.5.2.</strong> Prover</a></li><li class="chapter-item "><a href="l2/aligned_mode.html"><strong aria-hidden="true">3.5.3.</strong> Aligned mode</a></li><li class="chapter-item "><a href="l2/tdx.html"><strong aria-hidden="true">3.5.4.</strong> TDX execution module</a></li></ol></li><li class="chapter-item "><a href="l2/roadmap.html"><strong aria-hidden="true">3.6.</strong> Based roadmap (draft)</a></li></ol></li><li class="chapter-item "><a href="developers/index.html"><strong aria-hidden="true">4.</strong> Developers</a><a class="toggle"><div>❱</div></a></li><li><ol class="section"><li class="chapter-item "><a href="developers/quickstart.html"><strong aria-hidden="true">4.1.</strong> Quickstart</a></li><li class="chapter-item "><a href="developers/l2_load_tests.html"><strong aria-hidden="true">4.2.</strong> L2 load-tests</a></li></ol></li><li class="chapter-item "><div><strong aria-hidden="true">5.</strong> CLI reference</div></li><li class="chapter-item "><div><strong aria-hidden="true">6.</strong> Troubleshooting</div></li><li class="chapter-item affix "><li class="part-title">Other resources</li><li class="chapter-item "><a href="perf_reports/index.html"><strong aria-hidden="true">7.</strong> Performance reports</a><a class="toggle"><div>❱</div></a></li><li><ol class="section"><li class="chapter-item "><a href="perf_reports/2025-04-03.html"><strong aria-hidden="true">7.1.</strong> 2025-04-03</a></li></ol></li></ol>';
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
