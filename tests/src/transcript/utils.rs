// ---------------------
// | META TEST HELPERS |
// ---------------------

use dojo_test_utils::sequencer::TestSequencer;
use eyre::Result;
use merlin::HashChainTranscript;
use mpc_stark::algebra::{scalar::Scalar, stark_curve::StarkPoint};
use once_cell::sync::OnceCell;
use starknet::core::{
    types::{DeclareTransactionResult, FieldElement},
    utils::cairo_short_string_to_felt,
};
use starknet_scripts::commands::utils::{
    calculate_contract_address, declare, deploy, get_artifacts, ScriptAccount,
};
use std::env;
use tracing::debug;

use crate::utils::{
    call_contract, global_setup, invoke_contract, scalar_to_felt, CalldataSerializable,
    ARTIFACTS_PATH_ENV_VAR, TRANSCRIPT_SEED,
};

pub const FUZZ_ROUNDS: usize = 10;

const TRANSCRIPT_WRAPPER_CONTRACT_NAME: &str = "renegade_contracts_TranscriptWrapper";

const RANGEPROOF_DOMAIN_SEP_FN_NAME: &str = "rangeproof_domain_sep";
const INNERPRODUCT_DOMAIN_SEP_FN_NAME: &str = "innerproduct_domain_sep";
const R1CS_DOMAIN_SEP_FN_NAME: &str = "r1cs_domain_sep";
const R1CS_1PHASE_DOMAIN_SEP_FN_NAME: &str = "r1cs_1phase_domain_sep";
const APPEND_SCALAR_FN_NAME: &str = "append_scalar";
const APPEND_POINT_FN_NAME: &str = "append_point";
const VALIDATE_AND_APPEND_POINT_FN_NAME: &str = "validate_and_append_point";
const CHALLENGE_SCALAR_FN_NAME: &str = "challenge_scalar";
const GET_CHALLENGE_SCALAR_FN_NAME: &str = "get_challenge_scalar";

pub static TRANSCRIPT_WRAPPER_ADDRESS: OnceCell<FieldElement> = OnceCell::new();

// ---------------------
// | META TEST HELPERS |
// ---------------------

pub async fn setup_transcript_test() -> Result<(TestSequencer, HashChainTranscript)> {
    let artifacts_path = env::var(ARTIFACTS_PATH_ENV_VAR).unwrap();

    let sequencer = global_setup().await;
    let account = sequencer.account();

    debug!("Declaring & deploying transcript wrapper contract...");
    let transcript_wrapper_address = deploy_transcript_wrapper(artifacts_path, &account).await?;
    if TRANSCRIPT_WRAPPER_ADDRESS.get().is_none() {
        // When running multiple tests, it's possible for the OnceCell to already be set.
        // However, we still want to deploy the contract, since each test gets its own sequencer.

        TRANSCRIPT_WRAPPER_ADDRESS
            .set(transcript_wrapper_address)
            .unwrap();
    }

    let hash_chain_transcript = HashChainTranscript::new(TRANSCRIPT_SEED.as_bytes());

    Ok((sequencer, hash_chain_transcript))
}

pub async fn deploy_transcript_wrapper(
    artifacts_path: String,
    account: &ScriptAccount,
) -> Result<FieldElement> {
    let (transcript_sierra_path, transcript_casm_path) =
        get_artifacts(&artifacts_path, TRANSCRIPT_WRAPPER_CONTRACT_NAME);
    let DeclareTransactionResult { class_hash, .. } =
        declare(transcript_sierra_path, transcript_casm_path, account).await?;

    let calldata = vec![
        cairo_short_string_to_felt(TRANSCRIPT_SEED)?,
        FieldElement::ZERO,
    ];
    deploy(account, class_hash, &calldata).await?;
    Ok(calculate_contract_address(class_hash, &calldata))
}

// --------------------------------
// | CONTRACT INTERACTION HELPERS |
// --------------------------------

pub async fn rangeproof_domain_sep(account: &ScriptAccount, n: u64, m: u64) -> Result<()> {
    let calldata = vec![FieldElement::from(n), FieldElement::from(m)];
    invoke_contract(
        account,
        *TRANSCRIPT_WRAPPER_ADDRESS.get().unwrap(),
        RANGEPROOF_DOMAIN_SEP_FN_NAME,
        calldata,
    )
    .await
}

pub async fn innerproduct_domain_sep(account: &ScriptAccount, n: u64) -> Result<()> {
    let calldata = vec![FieldElement::from(n)];
    invoke_contract(
        account,
        *TRANSCRIPT_WRAPPER_ADDRESS.get().unwrap(),
        INNERPRODUCT_DOMAIN_SEP_FN_NAME,
        calldata,
    )
    .await
}

pub async fn r1cs_domain_sep(account: &ScriptAccount) -> Result<()> {
    invoke_contract(
        account,
        *TRANSCRIPT_WRAPPER_ADDRESS.get().unwrap(),
        R1CS_DOMAIN_SEP_FN_NAME,
        vec![],
    )
    .await
}

pub async fn r1cs_1phase_domain_sep(account: &ScriptAccount) -> Result<()> {
    invoke_contract(
        account,
        *TRANSCRIPT_WRAPPER_ADDRESS.get().unwrap(),
        R1CS_1PHASE_DOMAIN_SEP_FN_NAME,
        vec![],
    )
    .await
}

pub async fn append_scalar(account: &ScriptAccount, label: &str, scalar: &Scalar) -> Result<()> {
    let calldata = vec![
        cairo_short_string_to_felt(label)?,
        FieldElement::ZERO,
        scalar_to_felt(scalar),
    ];
    invoke_contract(
        account,
        *TRANSCRIPT_WRAPPER_ADDRESS.get().unwrap(),
        APPEND_SCALAR_FN_NAME,
        calldata,
    )
    .await
}

pub async fn append_point(account: &ScriptAccount, label: &str, point: &StarkPoint) -> Result<()> {
    // We assume labels are less than 16 bytes long, so we can use cairo_short_string_to_felt for simplicity
    let mut calldata = vec![cairo_short_string_to_felt(label)?, FieldElement::ZERO];
    calldata.extend(point.to_calldata());
    invoke_contract(
        account,
        *TRANSCRIPT_WRAPPER_ADDRESS.get().unwrap(),
        APPEND_POINT_FN_NAME,
        calldata,
    )
    .await
}

pub async fn validate_and_append_point(
    account: &ScriptAccount,
    label: &str,
    point: &StarkPoint,
) -> Result<()> {
    // We assume labels are less than 16 bytes long, so we can use cairo_short_string_to_felt for simplicity
    let mut calldata = vec![cairo_short_string_to_felt(label)?, FieldElement::ZERO];
    calldata.extend(point.to_calldata());
    invoke_contract(
        account,
        *TRANSCRIPT_WRAPPER_ADDRESS.get().unwrap(),
        VALIDATE_AND_APPEND_POINT_FN_NAME,
        calldata,
    )
    .await
}

pub async fn challenge_scalar(account: &ScriptAccount, label: &str) -> Result<()> {
    let calldata = vec![cairo_short_string_to_felt(label)?, FieldElement::ZERO];
    invoke_contract(
        account,
        *TRANSCRIPT_WRAPPER_ADDRESS.get().unwrap(),
        CHALLENGE_SCALAR_FN_NAME,
        calldata,
    )
    .await
}

pub async fn get_challenge_scalar(account: &ScriptAccount) -> Result<Scalar> {
    call_contract(
        account,
        *TRANSCRIPT_WRAPPER_ADDRESS.get().unwrap(),
        GET_CHALLENGE_SCALAR_FN_NAME,
        vec![],
    )
    .await
    .map(|r| Scalar::from_be_bytes_mod_order(&r[0].to_bytes_be()))
}
