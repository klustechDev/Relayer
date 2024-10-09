// scripts/deploy.js

async function main() {
    const [deployer] = await ethers.getSigners();
    console.log("Deploying the RelayerExploit contract with the account:", deployer.address);

    const RelayerExploit = await ethers.getContractFactory("RelayerExploit");
    const exploit = await RelayerExploit.deploy();
    await exploit.deployed();

    console.log("RelayerExploit deployed to:", exploit.address);
}

main()
  .then(() => process.exit(0))
  .catch(error => {
      console.error("Error during deployment:", error);
      process.exit(1);
  });
