const { expect } = require("chai");
const { ethers, upgrades } = require("hardhat");

describe("Upgrade validation (local example)", function () {
  it("accepts compatible upgrades and rejects incompatible ones", async function () {
    const Box = await ethers.getContractFactory("BoxUpgradeable");
    const BoxV2Good = await ethers.getContractFactory("BoxUpgradeableV2Good");
    const BoxV2Bad = await ethers.getContractFactory("BoxUpgradeableV2Bad");

    await upgrades.validateUpgrade(Box, BoxV2Good, { kind: "uups" });
    await expect(
      upgrades.validateUpgrade(Box, BoxV2Bad, { kind: "uups" })
    ).to.be.rejected;
  });
});
