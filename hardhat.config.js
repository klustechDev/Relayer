require("@nomiclabs/hardhat-ethers");

module.exports = {
  solidity: "0.8.0",
  paths: {
    sources: "./contracts-stylus/exploits",
    tests: "./test",
    cache: "./cache",
    artifacts: "./artifacts"
  },
  networks: {
    localhost: {
      url: "http://127.0.0.1:8545",
      chainId: 1337
    },
    hardhat: {
      chainId: 1337
    }
  }
};
