// scripts/checkState.js

async function main() {
    const [attacker] = await ethers.getSigners();
    console.log("Checking contract state with account:", attacker.address);

    const exploitAddress = "0xa513E6E4b8f2a923D98304ec87F64353C4D5C853";
    const RelayerExploit = await ethers.getContractFactory("RelayerExploit");
    const exploitContract = await RelayerExploit.attach(exploitAddress);

    try {
        const [lastCommitment, lastAmount] = await exploitContract.getLastManipulatedCommitment();
        console.log("Last manipulated commitment:", lastCommitment);
        console.log("Last manipulated amount:", ethers.utils.formatEther(lastAmount));
    } catch (error) {
        console.error("Error in checkState script:", error);
    }
}

main()
    .then(() => process.exit(0))
    .catch((error) => {
        console.error("Error in checkState script:", error);
        process.exit(1);
    });
