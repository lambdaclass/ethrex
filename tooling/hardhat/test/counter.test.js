const { expect } = require("chai");
const { ethers } = require("hardhat");

describe("Counter", function () {
  it("increments", async function () {
    const Counter = await ethers.getContractFactory("Counter");
    const counter = await Counter.deploy();
    await counter.waitForDeployment();

    expect(await counter.count()).to.equal(0n);

    const tx = await counter.increment();
    await tx.wait();

    expect(await counter.count()).to.equal(1n);
  });
});
