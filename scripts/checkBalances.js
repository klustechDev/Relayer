// scripts/checkBalances.js

async function main() {
    const [deployer] = await ethers.getSigners();
    console.log("Checking balances with account:", deployer.address);

    const exploitAddress = "0x5FC8d32690cc91D4c39d9d3abcBD16989F875707";

    const balanceBefore = await ethers.provider.getBalance(exploitAddress);
    console.log("Contract balance before exploit:", ethers.utils.formatEther(balanceBefore));

    // Here you may re-run the exploit or observe balances post-exploit
    const balanceAfter = await ethers.provider.getBalance(exploitAddress);
    console.log("Contract balance after exploit:", ethers.utils.formatEther(balanceAfter));
}

main()
    .then(() => process.exit(0))
    .catch((error) => {
        console.error("Error in checkBalances script:", error);
        process.exit(1);
    });
