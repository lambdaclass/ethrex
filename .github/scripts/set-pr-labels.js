module.exports = async ({ github, context }) => {
  const title = context.payload.pull_request?.title || "";
  const titleLc = title.toLowerCase();

  // Work internally with lowercase labels for consistency
  const desired = new Set(); // lowercase names
  const managed = new Set(["l1", "l2", "levm", "performance"]);

  // Add performance for perf type
  if (/^\s*perf\b/.test(titleLc)) {
    desired.add("performance");
  }

  // Extract scopes inside the first parentheses after type
  // Conventional format: type(scope[, scope2, ...]): subject
  const match = titleLc.match(/^[a-z]+\(([^)]+)\):/i);
  if (match) {
    const scopes = match[1]
      .split(',')
      .map((s) => s.trim())
      .filter(Boolean);

    for (const s of scopes) {
      if (s === 'l1') desired.add('l1');
      if (s === 'l2') desired.add('l2');
      if (s === 'levm') desired.add('levm');
    }
  }

  // Fetch existing labels on the PR
  const { data: existing } = await github.rest.issues.listLabelsOnIssue({
    owner: context.repo.owner,
    repo: context.repo.repo,
    issue_number: context.issue.number,
  });

  // Keep original names and a lowercase lookup map
  const existingLcMap = new Map(existing.map((l) => [l.name.toLowerCase(), l.name]));

  // Determine adds using lowercase comparisons
  const toAddLc = Array.from(desired).filter((lc) => !existingLcMap.has(lc));

  // Map lowercase desired names to canonical repo label names for API calls
  // Keep current repo conventions: 'L1'/'L2' are capitalized; others lowercase
  const canonical = (lc) => (lc === 'l1' ? 'L1' : lc === 'l2' ? 'L2' : lc);
  const toAdd = toAddLc.map(canonical);

  // Determine removals of managed labels that are present but no longer desired
  const toRemove = Array.from(existingLcMap.keys())
    .filter((lc) => managed.has(lc) && !desired.has(lc))
    .map((lc) => existingLcMap.get(lc)); // use original casing for removal

  if (toAdd.length > 0) {
    await github.rest.issues.addLabels({
      owner: context.repo.owner,
      repo: context.repo.repo,
      issue_number: context.issue.number,
      labels: toAdd,
    });
  }

  for (const name of toRemove) {
    await github.rest.issues.removeLabel({
      owner: context.repo.owner,
      repo: context.repo.repo,
      issue_number: context.issue.number,
      name,
    });
  }
};

