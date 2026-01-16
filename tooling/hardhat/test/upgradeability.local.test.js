const { expect } = require("chai");
const { ethers, upgrades } = require("hardhat");

describe("Upgrade validation (local example)", function () {
  it("accepts compatible upgrades and rejects incompatible ones", async function () {
    const Box = await ethers.getContractFactory("BoxUpgradeable");
    const BoxV2 = await ethers.getContractFactory("BoxUpgradeableV2");
    const BoxBad = await ethers.getContractFactory("BoxUpgradeableBad");

    await upgrades.validateUpgrade(Box, BoxV2, { kind: "uups" });
    await expect(
      upgrades.validateUpgrade(Box, BoxBad, { kind: "uups" })
    ).to.be.rejected;
  });
});
