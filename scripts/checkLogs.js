// scripts/checkLogs.js

async function main() {
    const [attacker] = await ethers.getSigners();
    console.log("Checking logs with account:", attacker.address);

    const exploitAddress = "0xa513E6E4b8f2a923D98304ec87F64353C4D5C853";
    const RelayerExploit = await ethers.getContractFactory("RelayerExploit");
    const exploitContract = await RelayerExploit.attach(exploitAddress);

    try {
        const filter = exploitContract.filters.CommitmentManipulated();
        const logs = await exploitContract.queryFilter(filter);
        console.log("Event Logs:", logs);
    } catch (error) {
        console.error("Error in checkLogs script:", error);
    }
}

main()
    .then(() => process.exit(0))
    .catch((error) => {
        console.error("Error in checkLogs script:", error);
        process.exit(1);
    });
