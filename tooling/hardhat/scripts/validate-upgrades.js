#!/usr/bin/env node
"use strict";

const path = require("path");
const fs = require("fs");
const {
  validateUpgradeSafety,
  ReferenceContractNotFound
} = require("@openzeppelin/upgrades-core");

function getArgValue(flag) {
  const index = process.argv.indexOf(flag);
  if (index === -1 || index + 1 >= process.argv.length) {
    return undefined;
  }
  return process.argv[index + 1];
}

function ensureDirExists(dir, label) {
  if (!fs.existsSync(dir)) {
    throw new Error(`${label} directory not found: ${dir}`);
  }
}

async function main() {
  const referenceDir =
    getArgValue("--reference") ||
    getArgValue("-r") ||
    process.env.UPGRADE_REFERENCE_BUILD_INFO_DIR;

  if (!referenceDir) {
    throw new Error(
      "Missing reference build-info directory. Use --reference <dir> or set UPGRADE_REFERENCE_BUILD_INFO_DIR."
    );
  }

  const buildInfoDir = path.join(__dirname, "..", "artifacts", "build-info");
  ensureDirExists(buildInfoDir, "Current build-info");
  ensureDirExists(referenceDir, "Reference build-info");

  const referenceKey = path.basename(referenceDir);

  const exclude = [
    "crates/l2/contracts/src/example/**",
    "fixtures/contracts/upgradeability/**",
    "@openzeppelin/**",
    "**/node_modules/**"
  ];

  const discoveryReport = await validateUpgradeSafety(
    buildInfoDir,
    undefined,
    undefined,
    {},
    [referenceDir],
    exclude
  );

  const upgradeable = discoveryReport.upgradeableContractReports.map(
    (report) => report.contract
  );

  if (upgradeable.length === 0) {
    console.log("No upgradeable contracts detected.");
    return;
  }

  const failures = [];
  const skipped = [];

  for (const contract of upgradeable) {
    const reference = `${referenceKey}:${contract}`;
    try {
      const report = await validateUpgradeSafety(
        buildInfoDir,
        contract,
        reference,
        {},
        [referenceDir],
        exclude
      );
      if (!report.ok) {
        failures.push(report);
      }
    } catch (err) {
      if (err instanceof ReferenceContractNotFound) {
        skipped.push(contract);
        continue;
      }
      throw err;
    }
  }

  if (skipped.length > 0) {
    console.log(
      `Skipped ${skipped.length} upgradeable contract(s) missing in reference: ${skipped.join(
        ", "
      )}`
    );
  }

  if (failures.length > 0) {
    console.error("Upgrade compatibility check failed:\n");
    for (const report of failures) {
      console.error(report.explain(false));
      console.error("\n");
    }
    process.exitCode = 1;
    return;
  }

  const validatedCount = upgradeable.length - skipped.length;

  console.log(
    `Upgrade compatibility check passed for ${validatedCount} contract(s).`
  );
}

main().catch((err) => {
  console.error(err);
  process.exitCode = 1;
});
