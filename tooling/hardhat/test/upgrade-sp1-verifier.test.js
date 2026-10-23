const { expect } = require("chai");
const { artifacts, ethers, network, upgrades } = require("hardhat");

const PROPOSERS = [
  {
    name: "OnChainProposer",
    fqn: "crates/l2/contracts/src/l1/OnChainProposer.sol:OnChainProposer",
    initArgs: ({ owner, sp1Verifier, aligned, bridge }) => [
      false, // validium
      owner,
      false, // requireRisc0Proof
      true, // requireSp1Proof
      false, // requireTdxProof
      false, // requireZiskProof
      aligned,
      ethers.ZeroAddress, // r0verifier
      sp1Verifier,
      ethers.ZeroAddress, // tdxverifier
      ethers.ZeroAddress, // ziskVerifier
      ethers.ZeroAddress, // alignedProofAggregator
      ethers.id("sp1 vk"),
      ethers.ZeroHash, // risc0Vk
      ethers.ZeroHash, // ziskVk
      ethers.id("commit"),
      ethers.id("genesis"),
      1729, // chainId
      bridge,
    ],
  },
  {
    name: "based OnChainProposer",
    fqn: "crates/l2/contracts/src/l1/based/OnChainProposer.sol:OnChainProposer",
    initArgs: ({ owner, sp1Verifier, aligned, bridge, sequencerRegistry }) => [
      false, // validium
      owner,
      false, // requireRisc0Proof
      true, // requireSp1Proof
      false, // requireTdxProof
      false, // requireZiskProof
      aligned,
      ethers.ZeroAddress, // r0verifier
      sp1Verifier,
      ethers.ZeroAddress, // tdxverifier
      ethers.ZeroAddress, // ziskVerifier
      ethers.ZeroAddress, // alignedProofAggregator
      ethers.id("sp1 vk"),
      ethers.ZeroHash, // risc0Vk
      ethers.ZeroHash, // ziskVk
      ethers.id("commit"),
      ethers.id("genesis"),
      sequencerRegistry,
      1729, // chainId
      bridge,
    ],
  },
];

// Committing batches needs a bridge, blobs and a sequencer, none of which matter
// to the verifier swap, so the tests write the two batch counters directly, at the
// slots the compiler reports for them.
async function setBatches(proposer, fqn, verified, committed) {
  const [source, name] = fqn.split(":");
  const buildInfo = await artifacts.getBuildInfo(fqn);
  const { storage } = buildInfo.output.contracts[source][name].storageLayout;
  for (const [label, value] of [
    ["lastVerifiedBatch", verified],
    ["lastCommittedBatch", committed],
  ]) {
    const entry = storage.find((candidate) => candidate.label === label);
    expect(entry.offset).to.equal(0);
    await network.provider.send("hardhat_setStorageAt", [
      await proposer.getAddress(),
      ethers.toQuantity(BigInt(entry.slot)),
      ethers.zeroPadValue(ethers.toBeHex(value), 32),
    ]);
  }
  expect(await proposer.lastVerifiedBatch()).to.equal(BigInt(verified));
  expect(await proposer.lastCommittedBatch()).to.equal(BigInt(committed));
}

for (const { name, fqn, initArgs } of PROPOSERS) {
  describe(`${name} upgradeSP1Verifier`, function () {
    let factory, proposer, owner, stranger, oldVerifier, newVerifier;

    async function deployProposer({ sp1Verifier, aligned = false }) {
      const deployed = await upgrades.deployProxy(factory, {
        kind: "uups",
        initializer: false,
      });
      await deployed.waitForDeployment();
      await (
        await deployed.initialize(
          ...initArgs({
            owner: owner.address,
            sp1Verifier,
            aligned,
            bridge: stranger.address,
            sequencerRegistry: stranger.address,
          })
        )
      ).wait();
      return deployed;
    }

    beforeEach(async function () {
      [owner, stranger] = await ethers.getSigners();
      factory = await ethers.getContractFactory(fqn);
      // Any contract stands in for a verifier: the setter only checks it has code.
      const Counter = await ethers.getContractFactory("Counter");
      oldVerifier = await (await Counter.deploy()).getAddress();
      newVerifier = await (await Counter.deploy()).getAddress();
      proposer = await deployProposer({ sp1Verifier: oldVerifier });
    });

    it("replaces the verifier once every committed batch is verified", async function () {
      await setBatches(proposer, fqn, 5, 5);
      await expect(proposer.upgradeSP1Verifier(newVerifier))
        .to.emit(proposer, "SP1VerifierUpgraded")
        .withArgs(newVerifier);
      expect(await proposer.SP1_VERIFIER_ADDRESS()).to.equal(newVerifier);
    });

    it("replaces the verifier before any batch is committed", async function () {
      await proposer.upgradeSP1Verifier(newVerifier);
      expect(await proposer.SP1_VERIFIER_ADDRESS()).to.equal(newVerifier);
    });

    it("refuses while a committed batch is unverified", async function () {
      await setBatches(proposer, fqn, 3, 5);
      await expect(proposer.upgradeSP1Verifier(newVerifier))
        .to.be.revertedWithCustomError(proposer, "UnverifiedBatchesPending")
        .withArgs(3, 5);
      expect(await proposer.SP1_VERIFIER_ADDRESS()).to.equal(oldVerifier);
    });

    it("is only callable by the owner", async function () {
      await expect(proposer.connect(stranger).upgradeSP1Verifier(newVerifier))
        .to.be.revertedWithCustomError(proposer, "OwnableUnauthorizedAccount")
        .withArgs(stranger.address);
    });

    it("refuses address zero while SP1 proofs are required", async function () {
      await expect(
        proposer.upgradeSP1Verifier(ethers.ZeroAddress)
      ).to.be.revertedWithCustomError(proposer, "MissingSp1Verifier");
    });

    it("allows address zero in Aligned mode", async function () {
      const aligned = await deployProposer({
        sp1Verifier: ethers.ZeroAddress,
        aligned: true,
      });
      await aligned.upgradeSP1Verifier(ethers.ZeroAddress);
      expect(await aligned.SP1_VERIFIER_ADDRESS()).to.equal(ethers.ZeroAddress);
    });

    it("refuses an address without code", async function () {
      await expect(
        proposer.upgradeSP1Verifier(stranger.address)
      ).to.be.revertedWithCustomError(proposer, "Sp1VerifierHasNoCode");
    });

    it("sets the verifier in the same call as an implementation upgrade", async function () {
      const newImplementation = await (await factory.deploy()).getAddress();
      await proposer.upgradeToAndCall(
        newImplementation,
        proposer.interface.encodeFunctionData("upgradeSP1Verifier", [newVerifier])
      );
      expect(
        await upgrades.erc1967.getImplementationAddress(await proposer.getAddress())
      ).to.equal(newImplementation);
      expect(await proposer.SP1_VERIFIER_ADDRESS()).to.equal(newVerifier);
    });

    it("reverts the whole implementation upgrade while a batch is unverified", async function () {
      const implementation = await upgrades.erc1967.getImplementationAddress(
        await proposer.getAddress()
      );
      const newImplementation = await (await factory.deploy()).getAddress();
      await setBatches(proposer, fqn, 3, 5);
      await expect(
        proposer.upgradeToAndCall(
          newImplementation,
          proposer.interface.encodeFunctionData("upgradeSP1Verifier", [newVerifier])
        )
      )
        .to.be.revertedWithCustomError(proposer, "UnverifiedBatchesPending")
        .withArgs(3, 5);
      expect(
        await upgrades.erc1967.getImplementationAddress(await proposer.getAddress())
      ).to.equal(implementation);
      expect(await proposer.SP1_VERIFIER_ADDRESS()).to.equal(oldVerifier);
    });

    it("refuses to initialize without an SP1 verifier while SP1 proofs are required", async function () {
      const uninitialized = await upgrades.deployProxy(factory, {
        kind: "uups",
        initializer: false,
      });
      await expect(
        uninitialized.initialize(
          ...initArgs({
            owner: owner.address,
            sp1Verifier: ethers.ZeroAddress,
            aligned: false,
            bridge: stranger.address,
            sequencerRegistry: stranger.address,
          })
        )
      ).to.be.revertedWithCustomError(uninitialized, "MissingSp1Verifier");
    });
  });
}
