//! Implementations of the various deploy scripts

use alloy_primitives::U256;
use circuits::zk_circuits::{
    valid_commitments::SizedValidCommitments, valid_match_settle::SizedValidMatchSettle,
    valid_reblind::SizedValidReblind, valid_wallet_create::SizedValidWalletCreate,
    valid_wallet_update::SizedValidWalletUpdate,
};
use constants::SystemCurve;
use contracts_utils::proof_system::{
    dummy_renegade_circuits::{
        DummyValidCommitments, DummyValidMatchSettle, DummyValidReblind, DummyValidWalletCreate,
        DummyValidWalletUpdate,
    },
    gen_circuit_vkey, gen_match_linking_vkeys, gen_match_vkeys,
};
use ethers::{
    abi::{Address, Contract},
    middleware::contract::ContractFactory,
    providers::Middleware,
    types::{Bytes, H256, U256 as EthersU256},
    utils::hex::FromHex,
};
use mpc_plonk::proof_system::{PlonkKzgSnark, UniversalSNARK};
use rand::thread_rng;
use std::{str::FromStr, sync::Arc};
use tracing::log::{info, warn};

use crate::{
    cli::{
        DeployErc20sArgs, DeployProxyArgs, DeployStylusArgs, DeployTestContractsArgs, GenSrsArgs,
        GenVkeysArgs, StylusContract, UpgradeArgs,
    },
    constants::{
        DARKPOOL_PROXY_ADMIN_CONTRACT_KEY, DARKPOOL_PROXY_CONTRACT_KEY, NUM_BYTES_ADDRESS,
        NUM_BYTES_STORAGE_SLOT, NUM_DEPLOY_CONFIRMATIONS, PROCESS_MATCH_SETTLE_VKEYS_FILE,
        PROXY_ABI, PROXY_ADMIN_STORAGE_SLOT, PROXY_BYTECODE, VALID_WALLET_CREATE_VKEY_FILE,
        VALID_WALLET_UPDATE_VKEY_FILE, VERIFIER_CONTRACT_KEY, VKEYS_CONTRACT_KEY,
    },
    errors::ScriptError,
    solidity::{DummyErc20Contract, ProxyAdminContract},
    utils::{
        build_stylus_contract, darkpool_initialize_calldata, deploy_stylus_contract,
        get_contract_key, parse_addr_from_deployments_file, parse_srs_from_file, setup_client,
        write_deployed_address, write_srs_to_file, write_vkey_file,
    },
};

/// Builds & deploys all of the contracts necessary for running the integration testing suite.
///
/// This includes generating fresh verification keys for testing.
pub async fn deploy_test_contracts(
    args: DeployTestContractsArgs,
    rpc_url: &str,
    priv_key: &str,
    client: Arc<impl Middleware>,
    deployments_path: &str,
) -> Result<(), ScriptError> {
    info!("Generating testing verification keys");
    let gen_vkeys_args = GenVkeysArgs {
        srs_path: args.srs_path.clone(),
        vkeys_dir: args.vkeys_dir.clone(),
        test: true,
    };
    gen_vkeys(gen_vkeys_args)?;

    let mut deploy_stylus_args = DeployStylusArgs {
        contract: StylusContract::TestVkeys,
        no_verify: args.no_verify,
    };

    info!("Deploying testing verification keys");
    build_and_deploy_stylus_contract(
        deploy_stylus_args,
        rpc_url,
        priv_key,
        client.clone(),
        deployments_path,
    )
    .await?;

    // Deploy the auxiliary testing contracts.
    // We do this first because they use the same compiler flags,
    // so we make use of the cached build artifacts.

    info!("Deploying dummy ERC-20 contract");
    deploy_stylus_args.contract = StylusContract::DummyErc20;
    build_and_deploy_stylus_contract(
        deploy_stylus_args,
        rpc_url,
        priv_key,
        client.clone(),
        deployments_path,
    )
    .await?;

    info!("Deploying dummy upgrade target contract");
    deploy_stylus_args.contract = StylusContract::DummyUpgradeTarget;
    build_and_deploy_stylus_contract(
        deploy_stylus_args,
        rpc_url,
        priv_key,
        client.clone(),
        deployments_path,
    )
    .await?;

    info!("Deploying precompiles testing contract");
    deploy_stylus_args.contract = StylusContract::PrecompileTestContract;
    build_and_deploy_stylus_contract(
        deploy_stylus_args,
        rpc_url,
        priv_key,
        client.clone(),
        deployments_path,
    )
    .await?;

    info!("Deploying Merkle testing contract");
    deploy_stylus_args.contract = StylusContract::MerkleTestContract;
    build_and_deploy_stylus_contract(
        deploy_stylus_args,
        rpc_url,
        priv_key,
        client.clone(),
        deployments_path,
    )
    .await?;

    info!("Deploying verifier contract");
    deploy_stylus_args.contract = StylusContract::Verifier;
    build_and_deploy_stylus_contract(
        deploy_stylus_args,
        rpc_url,
        priv_key,
        client.clone(),
        deployments_path,
    )
    .await?;

    info!("Deploying darkpool testing contract");
    deploy_stylus_args.contract = StylusContract::DarkpoolTestContract;
    build_and_deploy_stylus_contract(
        deploy_stylus_args,
        rpc_url,
        priv_key,
        client.clone(),
        deployments_path,
    )
    .await?;

    info!("Deploying proxy contract");
    let deploy_proxy_args = DeployProxyArgs {
        owner: args.owner,
        fee: args.fee,
    };
    deploy_proxy(deploy_proxy_args, client, deployments_path).await?;

    Ok(())
}

/// Deploys the `TransparentUpgradeableProxy` and `ProxyAdmin` contracts
pub async fn deploy_proxy(
    args: DeployProxyArgs,
    client: Arc<impl Middleware>,
    deployments_path: &str,
) -> Result<(), ScriptError> {
    // Get proxy contract ABI and bytecode
    let abi: Contract =
        serde_json::from_str(PROXY_ABI).map_err(|e| ScriptError::ArtifactParsing(e.to_string()))?;

    let bytecode =
        Bytes::from_hex(PROXY_BYTECODE).map_err(|e| ScriptError::ArtifactParsing(e.to_string()))?;

    let proxy_factory = ContractFactory::new(abi, bytecode, client.clone());

    // Parse proxy contract constructor arguments

    let darkpool_address = parse_addr_from_deployments_file(
        deployments_path,
        get_contract_key(StylusContract::Darkpool),
    )?;
    let merkle_address = parse_addr_from_deployments_file(
        deployments_path,
        get_contract_key(StylusContract::Merkle),
    )?;
    let verifier_address =
        parse_addr_from_deployments_file(deployments_path, VERIFIER_CONTRACT_KEY)?;

    let vkeys_address = parse_addr_from_deployments_file(deployments_path, VKEYS_CONTRACT_KEY)?;

    let owner_address = Address::from_str(&args.owner)
        .map_err(|e| ScriptError::CalldataConstruction(e.to_string()))?;

    let protocol_fee = U256::from(args.fee);

    let darkpool_calldata = Bytes::from(darkpool_initialize_calldata(
        verifier_address,
        vkeys_address,
        merkle_address,
        protocol_fee,
    )?);

    info!(
        "Deploying proxy using:\n\tDarkpool address: {:#x}\n\tMerkle address: {:#x}\n\tVerifier address: {:#x}\n\tVkeys address: {:#x}",
        darkpool_address, merkle_address, verifier_address, vkeys_address
    );

    // Deploy proxy contract
    let proxy_contract = proxy_factory
        .deploy((darkpool_address, owner_address, darkpool_calldata))
        .map_err(|e| ScriptError::ContractDeployment(e.to_string()))?
        .confirmations(NUM_DEPLOY_CONFIRMATIONS)
        .send()
        .await
        .map_err(|e| ScriptError::ContractDeployment(e.to_string()))?;

    let proxy_address = proxy_contract.address();

    info!(
        "Proxy contract deployed at address:\n\t{:#x}",
        proxy_address
    );

    // Get proxy admin contract address
    // This is the recommended way to get the proxy admin address:
    // https://github.com/OpenZeppelin/openzeppelin-contracts/blob/v5.0.0/contracts/proxy/ERC1967/ERC1967Utils.sol#L104-L106
    let proxy_admin_address = Address::from_slice(
        &client
            .get_storage_at(
                proxy_address,
                // Can `unwrap` here since we know the storage slot constitutes a valid H256
                H256::from_str(PROXY_ADMIN_STORAGE_SLOT).unwrap(),
                None, /* block */
            )
            .await
            .map_err(|e| ScriptError::ContractInteraction(e.to_string()))?
            [NUM_BYTES_STORAGE_SLOT - NUM_BYTES_ADDRESS..NUM_BYTES_STORAGE_SLOT],
    );

    info!(
        "Proxy admin contract deployed at address:\n\t{:#x}",
        proxy_admin_address
    );

    // Write deployed addresses to deployments file
    write_deployed_address(deployments_path, DARKPOOL_PROXY_CONTRACT_KEY, proxy_address)?;
    write_deployed_address(
        deployments_path,
        DARKPOOL_PROXY_ADMIN_CONTRACT_KEY,
        proxy_admin_address,
    )?;

    Ok(())
}

/// Deploys the ERC-20 contracts & approves the darkpool
/// to spend the maximum amount of tokens for the provided
/// addresses.
///
/// Note: the provided tickers will not actually be used as the contract's
/// name or symbol, but rather as a way to identify the contract in the deployments file.
pub async fn deploy_erc20s(
    args: DeployErc20sArgs,
    rpc_url: &str,
    priv_key: &str,
    client: Arc<impl Middleware>,
    deployments_path: &str,
) -> Result<(), ScriptError> {
    let wasm_file_path =
        build_stylus_contract(StylusContract::DummyErc20, false /* no_verify */)?;

    let mut erc20_addresses = Vec::with_capacity(args.tickers.len());
    for ticker in args.tickers {
        erc20_addresses.push(
            deploy_stylus_contract(
                wasm_file_path.clone(),
                rpc_url,
                priv_key,
                client.clone(),
                StylusContract::DummyErc20,
                deployments_path,
                Some(&ticker),
            )
            .await?,
        );
    }

    let darkpool_address =
        parse_addr_from_deployments_file(deployments_path, DARKPOOL_PROXY_CONTRACT_KEY)?;

    for erc20_address in erc20_addresses {
        for skey in &args.approval_skeys {
            let approval_client = setup_client(&skey, rpc_url).await?;
            let erc20 = DummyErc20Contract::new(erc20_address, approval_client);
            erc20
                .approve(darkpool_address, EthersU256::MAX)
                .send()
                .await
                .map_err(|e| ScriptError::ContractInteraction(e.to_string()))?
                .await
                .map_err(|e| ScriptError::ContractInteraction(e.to_string()))?;
        }
    }

    Ok(())
}

/// Builds and deploys a Stylus contract
pub async fn build_and_deploy_stylus_contract(
    args: DeployStylusArgs,
    rpc_url: &str,
    priv_key: &str,
    client: Arc<impl Middleware>,
    deployments_path: &str,
) -> Result<(), ScriptError> {
    let wasm_file_path = build_stylus_contract(args.contract, args.no_verify)?;
    deploy_stylus_contract(
        wasm_file_path,
        rpc_url,
        priv_key,
        client,
        args.contract,
        deployments_path,
        None,
    )
    .await
    .map(|_| ())
}

/// Upgrades the darkpool implementation
pub async fn upgrade(
    args: UpgradeArgs,
    client: Arc<impl Middleware>,
    deployments_path: &str,
) -> Result<(), ScriptError> {
    let proxy_admin_address =
        parse_addr_from_deployments_file(deployments_path, DARKPOOL_PROXY_ADMIN_CONTRACT_KEY)?;
    let proxy_admin = ProxyAdminContract::new(proxy_admin_address, client);

    let proxy_address =
        parse_addr_from_deployments_file(deployments_path, DARKPOOL_PROXY_CONTRACT_KEY)?;

    let implementation_address = parse_addr_from_deployments_file(
        deployments_path,
        get_contract_key(StylusContract::Darkpool),
    )?;

    let data = if let Some(calldata) = args.calldata {
        Bytes::from_hex(calldata).map_err(|e| ScriptError::CalldataConstruction(e.to_string()))?
    } else {
        Bytes::new()
    };

    proxy_admin
        .upgrade_and_call(proxy_address, implementation_address, data)
        .send()
        .await
        .map_err(|e| ScriptError::ContractInteraction(e.to_string()))?
        .await
        .map_err(|e| ScriptError::ContractInteraction(e.to_string()))?;

    Ok(())
}

/// Generates a structured reference string
pub fn gen_srs(args: GenSrsArgs) -> Result<(), ScriptError> {
    let mut rng = thread_rng();

    // Generate universal SRS
    warn!("Generating UNSAFE universal SRS, should only be used in testing");
    let srs = PlonkKzgSnark::<SystemCurve>::universal_setup_for_testing(args.degree, &mut rng)
        .map_err(|e| ScriptError::SrsGeneration(e.to_string()))?;

    write_srs_to_file(&args.srs_path, &srs)
}

/// Generates verification keys for the protocol circuits
pub fn gen_vkeys(args: GenVkeysArgs) -> Result<(), ScriptError> {
    let srs = parse_srs_from_file(&args.srs_path)?;

    let (valid_wallet_create_vkey, valid_wallet_update_vkey, match_vkeys, match_linking_vkeys) =
        if args.test {
            (
                gen_circuit_vkey::<DummyValidWalletCreate>(&srs)
                    .map_err(|_| ScriptError::CircuitCreation)?,
                gen_circuit_vkey::<DummyValidWalletUpdate>(&srs)
                    .map_err(|_| ScriptError::CircuitCreation)?,
                gen_match_vkeys::<DummyValidCommitments, DummyValidReblind, DummyValidMatchSettle>(
                    &srs,
                )
                .map_err(|_| ScriptError::CircuitCreation)?,
                gen_match_linking_vkeys::<DummyValidCommitments>()
                    .map_err(|_| ScriptError::CircuitCreation)?,
            )
        } else {
            (
                gen_circuit_vkey::<SizedValidWalletCreate>(&srs)
                    .map_err(|_| ScriptError::CircuitCreation)?,
                gen_circuit_vkey::<SizedValidWalletUpdate>(&srs)
                    .map_err(|_| ScriptError::CircuitCreation)?,
                gen_match_vkeys::<SizedValidCommitments, SizedValidReblind, SizedValidMatchSettle>(
                    &srs,
                )
                .map_err(|_| ScriptError::CircuitCreation)?,
                gen_match_linking_vkeys::<SizedValidCommitments>()
                    .map_err(|_| ScriptError::CircuitCreation)?,
            )
        };

    let valid_wallet_create_vkey_bytes = postcard::to_allocvec(&valid_wallet_create_vkey)
        .map_err(|e| ScriptError::Serde(e.to_string()))?;
    let valid_wallet_update_vkey_bytes = postcard::to_allocvec(&valid_wallet_update_vkey)
        .map_err(|e| ScriptError::Serde(e.to_string()))?;

    let match_vkeys_bytes =
        postcard::to_allocvec(&match_vkeys).map_err(|e| ScriptError::Serde(e.to_string()))?;
    let match_linking_vkeys_bytes = postcard::to_allocvec(&match_linking_vkeys)
        .map_err(|e| ScriptError::Serde(e.to_string()))?;

    write_vkey_file(
        &args.vkeys_dir,
        VALID_WALLET_CREATE_VKEY_FILE,
        &valid_wallet_create_vkey_bytes,
    )?;
    write_vkey_file(
        &args.vkeys_dir,
        VALID_WALLET_UPDATE_VKEY_FILE,
        &valid_wallet_update_vkey_bytes,
    )?;

    // The match vkeys & linking vkeys are serialized together
    let process_match_settle_vkey_bytes = [match_vkeys_bytes, match_linking_vkeys_bytes].concat();

    write_vkey_file(
        &args.vkeys_dir,
        PROCESS_MATCH_SETTLE_VKEYS_FILE,
        &process_match_settle_vkey_bytes,
    )?;

    Ok(())
}
