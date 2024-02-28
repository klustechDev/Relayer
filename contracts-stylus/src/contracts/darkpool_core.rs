//! The darkpool core contract, containing all of the critical, wallet-modifying functionality.
//! This contract assumes it is being delegate-called by the "outer" darkpool contract and that
//! certain storage elements are set by the outer contract. As such, its storage layout must
//! exactly align with that of the outer contract.

use core::borrow::{Borrow, BorrowMut};

use crate::{
    assert_result, if_verifying,
    utils::{
        constants::{
            INVALID_ORDER_SETTLEMENT_INDICES_ERROR_MESSAGE, INVALID_PROTOCOL_FEE_ERROR_MESSAGE,
            INVALID_PROTOCOL_PUBKEY_ERROR_MESSAGE, MERKLE_STORAGE_GAP_SIZE,
            NULLIFIER_SPENT_ERROR_MESSAGE, ROOT_NOT_IN_HISTORY_ERROR_MESSAGE,
            TRANSFER_EXECUTOR_STORAGE_GAP_SIZE, VERIFICATION_FAILED_ERROR_MESSAGE,
        },
        helpers::{
            delegate_call_helper, deserialize_from_calldata, pk_to_u256s, postcard_serialize,
            serialize_match_statements_for_verification, serialize_statement_for_verification,
            static_call_helper, u256_to_scalar,
        },
        solidity::{
            executeExternalTransferCall, insertNoteCommitmentCall, insertSharesCommitmentCall,
            processMatchSettleVkeysCall, rootInHistoryCall, validFeeRedemptionVkeyCall,
            validOfflineFeeSettlementVkeyCall, validRelayerFeeSettlementVkeyCall,
            validWalletCreateVkeyCall, validWalletUpdateVkeyCall, verifyCall, verifyMatchCall,
            verifyStateSigAndInsertCall, NotePosted, NullifierSpent, WalletUpdated,
        },
    },
};
use alloc::{vec, vec::Vec};
use contracts_common::{
    custom_serde::scalar_to_u256,
    types::{
        ExternalTransfer, MatchPayload, PublicEncryptionKey, PublicSigningKey, ScalarField,
        ValidFeeRedemptionStatement, ValidMatchSettleStatement, ValidOfflineFeeSettlementStatement,
        ValidRelayerFeeSettlementStatement, ValidWalletCreateStatement, ValidWalletUpdateStatement,
    },
};
use stylus_sdk::{
    abi::Bytes,
    alloy_primitives::U256,
    evm,
    prelude::*,
    storage::{StorageAddress, StorageArray, StorageBool, StorageMap, StorageU256, StorageU64},
};

/// The darkpool core contract's storage layout.
/// This contract mirrors the storage elements from the "outer"
/// darkpool contract where they are set, so that they can be fetched
/// without a delegatecall.
/// Many storage elements are not used in the darkpool core contract,
/// but are listed here so that the storage layout lines up with
/// that of the darkpool contract.
#[solidity_storage]
#[cfg_attr(feature = "darkpool-core", entrypoint)]
pub struct DarkpoolCoreContract {
    /// Storage gap to prevent collisions with the Merkle contract
    __merkle_gap: StorageArray<StorageU256, MERKLE_STORAGE_GAP_SIZE>,

    /// Storage gap to prevent collisions with the transfer executor contract
    __transfer_executor_gap: StorageArray<StorageU256, TRANSFER_EXECUTOR_STORAGE_GAP_SIZE>,

    /// The owner of the darkpool contract
    /// (unused in the darkpool core contract)
    _owner: StorageAddress,

    /// Whether or not the darkpool has been initialized
    /// (unused in the darkpool core contract)
    _initialized: StorageU64,

    /// Whether or not the darkpool is paused
    /// (unused in the darkpool core contract)
    _paused: StorageBool,

    /// The address of the darkpool core contract
    /// (unused in the darkpool core contract)
    _darkpool_core_address: StorageAddress,

    /// The address of the verifier contract
    verifier_address: StorageAddress,

    /// The address of the vkeys contract
    vkeys_address: StorageAddress,

    /// The address of the Merkle contract
    merkle_address: StorageAddress,

    /// The address of the transfer executor contract
    transfer_executor_address: StorageAddress,

    /// The set of wallet nullifiers, representing a mapping from a nullifier
    /// (which is a Bn254 scalar field element serialized into 32 bytes) to a
    /// boolean indicating whether or not the nullifier is spent
    nullifier_set: StorageMap<U256, StorageBool>,

    /// The protocol fee, representing a percentage of the trade volume
    /// as a fixed-point number shifted by 32 bits.
    ///
    /// I.e., the fee is `protocol_fee / 2^32`
    protocol_fee: StorageU256,

    /// The BabyJubJub EC-ElGamal public encryption key for the protocol
    protocol_public_encryption_key: StorageArray<StorageU256, 2>,
}

#[external]
impl DarkpoolCoreContract {
    /// Adds a new wallet to the commitment tree
    pub fn new_wallet<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        proof: Bytes,
        valid_wallet_create_statement_bytes: Bytes,
    ) -> Result<(), Vec<u8>> {
        let valid_wallet_create_statement: ValidWalletCreateStatement =
            deserialize_from_calldata(&valid_wallet_create_statement_bytes)?;

        if_verifying!({
            let vkeys_address = storage.borrow_mut().vkeys_address.get();
            let (valid_wallet_create_vkey_bytes,) =
                static_call_helper::<validWalletCreateVkeyCall>(storage, vkeys_address, ())?.into();

            assert_result!(
                DarkpoolCoreContract::verify(
                    storage,
                    valid_wallet_create_vkey_bytes,
                    proof.into(),
                    serialize_statement_for_verification(&valid_wallet_create_statement)?,
                )?,
                VERIFICATION_FAILED_ERROR_MESSAGE
            )?;
        });

        DarkpoolCoreContract::insert_wallet_commitment_to_merkle_tree(
            storage,
            valid_wallet_create_statement.private_shares_commitment,
            &valid_wallet_create_statement.public_wallet_shares,
        )?;

        DarkpoolCoreContract::log_wallet_update(
            &valid_wallet_create_statement.public_wallet_shares,
        );

        Ok(())
    }

    /// Update a wallet in the commitment tree
    pub fn update_wallet<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        proof: Bytes,
        valid_wallet_update_statement_bytes: Bytes,
        wallet_commitment_signature: Bytes,
        transfer_aux_data_bytes: Bytes,
    ) -> Result<(), Vec<u8>> {
        let valid_wallet_update_statement: ValidWalletUpdateStatement =
            deserialize_from_calldata(&valid_wallet_update_statement_bytes)?;

        if_verifying!({
            let vkeys_address = storage.borrow_mut().vkeys_address.get();
            let (valid_wallet_update_vkey_bytes,) =
                static_call_helper::<validWalletUpdateVkeyCall>(storage, vkeys_address, ())?.into();

            assert_result!(
                DarkpoolCoreContract::verify(
                    storage,
                    valid_wallet_update_vkey_bytes,
                    proof.into(),
                    serialize_statement_for_verification(&valid_wallet_update_statement)?,
                )?,
                VERIFICATION_FAILED_ERROR_MESSAGE
            )?;
        });

        DarkpoolCoreContract::rotate_wallet_with_signature(
            storage,
            valid_wallet_update_statement.old_shares_nullifier,
            valid_wallet_update_statement.merkle_root,
            valid_wallet_update_statement.new_private_shares_commitment,
            &valid_wallet_update_statement.new_public_shares,
            wallet_commitment_signature.into(),
            valid_wallet_update_statement.old_pk_root,
        )?;

        if let Some(external_transfer) = valid_wallet_update_statement.external_transfer {
            DarkpoolCoreContract::execute_external_transfer(
                storage,
                valid_wallet_update_statement.old_pk_root,
                external_transfer,
                transfer_aux_data_bytes,
            )?;
        }

        Ok(())
    }

    /// Settles a matched order between two parties,
    /// inserting the updated wallets into the commitment tree.
    ///
    /// The `match_proofs` argument is the serialization of the [`contracts_common::types::MatchProofs`]
    /// struct, and the `match_linking_proofs` argument is the serialization of the
    /// [`contracts_common::types::MatchLinkingProofs`] struct
    pub fn process_match_settle<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        party_0_match_payload: Bytes,
        party_1_match_payload: Bytes,
        valid_match_settle_statement: Bytes,
        match_proofs: Bytes,
        match_linking_proofs: Bytes,
    ) -> Result<(), Vec<u8>> {
        let party_0_match_payload: MatchPayload =
            deserialize_from_calldata(&party_0_match_payload)?;

        let party_1_match_payload: MatchPayload =
            deserialize_from_calldata(&party_1_match_payload)?;

        let valid_match_settle_statement: ValidMatchSettleStatement =
            deserialize_from_calldata(&valid_match_settle_statement)?;

        if_verifying!({
            let party0_same_indices = party_0_match_payload.valid_commitments_statement.indices
                == valid_match_settle_statement.party0_indices;
            let party1_same_indices = party_1_match_payload.valid_commitments_statement.indices
                == valid_match_settle_statement.party1_indices;

            assert_result!(
                party0_same_indices && party1_same_indices,
                INVALID_ORDER_SETTLEMENT_INDICES_ERROR_MESSAGE
            )?;

            // We convert the protocol fee directly to a scalar as it is already kept
            // in storage as fixed-point number, no manipulation is needed to coerce it
            // to the form expected in the statement / circuit.
            let protocol_fee = u256_to_scalar(storage.borrow_mut().protocol_fee.get())?;
            assert_result!(
                valid_match_settle_statement.protocol_fee == protocol_fee,
                INVALID_PROTOCOL_FEE_ERROR_MESSAGE
            )?;

            DarkpoolCoreContract::batch_verify_process_match_settle(
                storage,
                &party_0_match_payload,
                &party_1_match_payload,
                &valid_match_settle_statement,
                match_proofs,
                match_linking_proofs,
            )?;
        });

        DarkpoolCoreContract::rotate_wallet(
            storage,
            party_0_match_payload
                .valid_reblind_statement
                .original_shares_nullifier,
            party_0_match_payload.valid_reblind_statement.merkle_root,
            party_0_match_payload
                .valid_reblind_statement
                .reblinded_private_shares_commitment,
            &valid_match_settle_statement.party0_modified_shares,
        )?;

        DarkpoolCoreContract::rotate_wallet(
            storage,
            party_1_match_payload
                .valid_reblind_statement
                .original_shares_nullifier,
            party_1_match_payload.valid_reblind_statement.merkle_root,
            party_1_match_payload
                .valid_reblind_statement
                .reblinded_private_shares_commitment,
            &valid_match_settle_statement.party1_modified_shares,
        )?;

        Ok(())
    }

    /// Settles the fee accumulated by a relayer for a given balance in a managed wallet
    /// into the relayer's wallet
    pub fn settle_online_relayer_fee<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        proof: Bytes,
        valid_relayer_fee_settlement_statement: Bytes,
        relayer_wallet_commitment_signature: Bytes,
    ) -> Result<(), Vec<u8>> {
        let valid_relayer_fee_settlement_statement: ValidRelayerFeeSettlementStatement =
            deserialize_from_calldata(&valid_relayer_fee_settlement_statement)?;

        if_verifying!({
            let vkeys_address = storage.borrow_mut().vkeys_address.get();
            let (valid_relayer_fee_settlement_vkey_bytes,) = static_call_helper::<
                validRelayerFeeSettlementVkeyCall,
            >(
                storage, vkeys_address, ()
            )?
            .into();

            assert_result!(
                DarkpoolCoreContract::verify(
                    storage,
                    valid_relayer_fee_settlement_vkey_bytes,
                    proof.into(),
                    serialize_statement_for_verification(&valid_relayer_fee_settlement_statement)?,
                )?,
                VERIFICATION_FAILED_ERROR_MESSAGE
            )?;
        });

        DarkpoolCoreContract::rotate_wallet(
            storage,
            valid_relayer_fee_settlement_statement.sender_nullifier,
            valid_relayer_fee_settlement_statement.sender_root,
            valid_relayer_fee_settlement_statement.sender_wallet_commitment,
            &valid_relayer_fee_settlement_statement.sender_updated_public_shares,
        )?;

        DarkpoolCoreContract::rotate_wallet_with_signature(
            storage,
            valid_relayer_fee_settlement_statement.recipient_nullifier,
            valid_relayer_fee_settlement_statement.recipient_root,
            valid_relayer_fee_settlement_statement.recipient_wallet_commitment,
            &valid_relayer_fee_settlement_statement.recipient_updated_public_shares,
            relayer_wallet_commitment_signature.into(),
            valid_relayer_fee_settlement_statement.recipient_pk_root,
        )
    }

    /// Settles the fee accumulated either by a relayer or the protocol
    /// into an encrypted note which is committed to the Merkle tree
    pub fn settle_offline_fee<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        proof: Bytes,
        valid_offline_fee_settlement_statement: Bytes,
    ) -> Result<(), Vec<u8>> {
        let valid_offline_fee_settlement_statement: ValidOfflineFeeSettlementStatement =
            deserialize_from_calldata(&valid_offline_fee_settlement_statement)?;

        if_verifying!({
            let protocol_pubkey =
                DarkpoolCoreContract::get_protocol_public_encryption_key(storage)?;
            assert_result!(
                valid_offline_fee_settlement_statement.protocol_key == protocol_pubkey,
                INVALID_PROTOCOL_PUBKEY_ERROR_MESSAGE
            )?;

            let vkeys_address = storage.borrow_mut().vkeys_address.get();
            let (valid_offline_fee_settlement_vkey_bytes,) = static_call_helper::<
                validOfflineFeeSettlementVkeyCall,
            >(
                storage, vkeys_address, ()
            )?
            .into();

            assert_result!(
                DarkpoolCoreContract::verify(
                    storage,
                    valid_offline_fee_settlement_vkey_bytes,
                    proof.into(),
                    serialize_statement_for_verification(&valid_offline_fee_settlement_statement)?,
                )?,
                VERIFICATION_FAILED_ERROR_MESSAGE
            )?;
        });

        DarkpoolCoreContract::rotate_wallet(
            storage,
            valid_offline_fee_settlement_statement.nullifier,
            valid_offline_fee_settlement_statement.merkle_root,
            valid_offline_fee_settlement_statement.updated_wallet_commitment,
            &valid_offline_fee_settlement_statement.updated_wallet_public_shares,
        )?;

        DarkpoolCoreContract::commit_note(
            storage,
            valid_offline_fee_settlement_statement.note_commitment,
        )
    }

    /// Redeems a fee note into the recipient's wallet, nullifying the note
    pub fn redeem_fee<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        proof: Bytes,
        valid_fee_redemption_statement: Bytes,
        recipient_wallet_commitment_signature: Bytes,
    ) -> Result<(), Vec<u8>> {
        let valid_fee_redemption_statement: ValidFeeRedemptionStatement =
            deserialize_from_calldata(&valid_fee_redemption_statement)?;

        if_verifying!({
            let vkeys_address = storage.borrow_mut().vkeys_address.get();
            let (valid_fee_redemption_vkey_bytes,) =
                static_call_helper::<validFeeRedemptionVkeyCall>(storage, vkeys_address, ())?
                    .into();

            assert_result!(
                DarkpoolCoreContract::verify(
                    storage,
                    valid_fee_redemption_vkey_bytes,
                    proof.into(),
                    serialize_statement_for_verification(&valid_fee_redemption_statement)?,
                )?,
                VERIFICATION_FAILED_ERROR_MESSAGE
            )?;
        });

        DarkpoolCoreContract::rotate_wallet_with_signature(
            storage,
            valid_fee_redemption_statement.nullifier,
            valid_fee_redemption_statement.wallet_root,
            valid_fee_redemption_statement.new_wallet_commitment,
            &valid_fee_redemption_statement.new_wallet_public_shares,
            recipient_wallet_commitment_signature.into(),
            valid_fee_redemption_statement.old_pk_root,
        )?;

        DarkpoolCoreContract::check_root_and_nullify(
            storage,
            valid_fee_redemption_statement.note_nullifier,
            valid_fee_redemption_statement.note_root,
        )
    }
}

impl DarkpoolCoreContract {
    // -----------------------
    // | CORE GETTER HELPERS |
    // -----------------------

    /// Gets the protocol public encryption key
    pub fn get_protocol_public_encryption_key<S: TopLevelStorage + Borrow<Self>>(
        storage: &S,
    ) -> Result<PublicEncryptionKey, Vec<u8>> {
        let protocol_pubkey_x = storage
            .borrow()
            .protocol_public_encryption_key
            .get(0)
            .unwrap();

        let protocol_pubkey_y = storage
            .borrow()
            .protocol_public_encryption_key
            .get(1)
            .unwrap();

        Ok(PublicEncryptionKey {
            x: u256_to_scalar(protocol_pubkey_x)?,
            y: u256_to_scalar(protocol_pubkey_y)?,
        })
    }

    /// Checks that the given Merkle root is in the root history
    pub fn check_root_in_history<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        root: ScalarField,
    ) -> Result<(), Vec<u8>> {
        let root = scalar_to_u256(root);
        let merkle_address = storage.borrow_mut().merkle_address.get();
        let (res,) =
            delegate_call_helper::<rootInHistoryCall>(storage, merkle_address, (root,))?.into();

        assert_result!(res, ROOT_NOT_IN_HISTORY_ERROR_MESSAGE)
    }

    // -----------------------
    // | CORE SETTER HELPERS |
    // -----------------------

    /// Marks the given nullifier as spent
    pub fn mark_nullifier_spent<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        nullifier: ScalarField,
    ) -> Result<(), Vec<u8>> {
        let this = storage.borrow_mut();

        let nullifier = scalar_to_u256(nullifier);

        if_verifying!(assert_result!(
            !this.nullifier_set.get(nullifier),
            NULLIFIER_SPENT_ERROR_MESSAGE
        )?);

        this.nullifier_set.insert(nullifier, true);

        evm::log(NullifierSpent { nullifier });
        Ok(())
    }

    /// Prepares the wallet shares for insertion into the Merkle tree by converting them
    /// to a vector of [`U256`]
    pub fn prepare_wallet_shares_for_insertion(
        private_shares_commitment: ScalarField,
        public_wallet_shares: &[ScalarField],
    ) -> Vec<U256> {
        let mut total_wallet_shares = vec![scalar_to_u256(private_shares_commitment)];
        for share in public_wallet_shares {
            total_wallet_shares.push(scalar_to_u256(*share));
        }
        total_wallet_shares
    }

    /// Prepares the private shares commitment & public wallet shares for insertion into the Merkle
    /// tree and delegate-calls the appropriate method on the Merkle contract
    pub fn insert_wallet_commitment_to_merkle_tree<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        private_shares_commitment: ScalarField,
        public_wallet_shares: &[ScalarField],
    ) -> Result<(), Vec<u8>> {
        let total_wallet_shares = Self::prepare_wallet_shares_for_insertion(
            private_shares_commitment,
            public_wallet_shares,
        );

        let merkle_address = storage.borrow_mut().merkle_address.get();
        delegate_call_helper::<insertSharesCommitmentCall>(
            storage,
            merkle_address,
            (total_wallet_shares,),
        )
        .map(|_| ())
    }

    /// Prepares the private shares commitment & public wallet shares for insertion into the Merkle
    /// tree, as well as the signature & pubkey for verification, and delegate-calls the appropriate
    /// method on the Merkle contract
    pub fn insert_signed_wallet_commitment_to_merkle_tree<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        private_shares_commitment: ScalarField,
        public_wallet_shares: &[ScalarField],
        wallet_commitment_signature: Vec<u8>,
        old_pk_root: &PublicSigningKey,
    ) -> Result<(), Vec<u8>> {
        let total_wallet_shares = Self::prepare_wallet_shares_for_insertion(
            private_shares_commitment,
            public_wallet_shares,
        );

        let merkle_address = storage.borrow_mut().merkle_address.get();

        let old_pk_root_u256s = pk_to_u256s(old_pk_root)?;

        delegate_call_helper::<verifyStateSigAndInsertCall>(
            storage,
            merkle_address,
            (
                total_wallet_shares,
                wallet_commitment_signature,
                old_pk_root_u256s,
            ),
        )
        .map(|_| ())
    }

    /// Verifies the given proof using the given public inputs
    /// & verification key.
    pub fn verify<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        vkey_ser: Vec<u8>,
        proof_ser: Vec<u8>,
        public_inputs_ser: Vec<u8>,
    ) -> Result<bool, Vec<u8>> {
        let this = storage.borrow_mut();
        let verifier_address = this.verifier_address.get();

        let verification_bundle_ser = [vkey_ser, proof_ser, public_inputs_ser].concat();

        let (result,) = static_call_helper::<verifyCall>(
            storage,
            verifier_address,
            (verification_bundle_ser,),
        )?
        .into();

        Ok(result)
    }

    /// Executes the given external transfer (withdrawal / deposit)
    pub fn execute_external_transfer<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        old_pk_root: PublicSigningKey,
        transfer: ExternalTransfer,
        transfer_aux_data_bytes: Bytes,
    ) -> Result<(), Vec<u8>> {
        let transfer_executor_address = storage.borrow_mut().transfer_executor_address.get();
        let old_pk_root_bytes = postcard_serialize(&Some(old_pk_root))?;
        let transfer_bytes = postcard_serialize(&transfer)?;

        delegate_call_helper::<executeExternalTransferCall>(
            storage,
            transfer_executor_address,
            (
                old_pk_root_bytes,
                transfer_bytes,
                transfer_aux_data_bytes.to_vec(),
            ),
        )?;

        Ok(())
    }

    /// Batch-verifies all of the `process_match_settle` proofs
    #[allow(clippy::too_many_arguments)]
    pub fn batch_verify_process_match_settle<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        party_0_match_payload: &MatchPayload,
        party_1_match_payload: &MatchPayload,
        valid_match_settle_statement: &ValidMatchSettleStatement,
        match_proofs: Bytes,
        match_linking_proofs: Bytes,
    ) -> Result<(), Vec<u8>> {
        let this = storage.borrow_mut();
        let vkeys_address = this.vkeys_address.get();
        let verifier_address = this.verifier_address.get();

        // Fetch the Plonk & linking verification keys used in verifying the matching of a trade
        let (process_match_settle_vkeys,) =
            static_call_helper::<processMatchSettleVkeysCall>(storage, vkeys_address, ())?.into();

        let match_public_inputs = serialize_match_statements_for_verification(
            &party_0_match_payload.valid_commitments_statement,
            &party_1_match_payload.valid_commitments_statement,
            &party_0_match_payload.valid_reblind_statement,
            &party_1_match_payload.valid_reblind_statement,
            valid_match_settle_statement,
        )?;

        let batch_verification_bundle_ser = [
            process_match_settle_vkeys,
            match_proofs.into(),
            match_public_inputs,
            match_linking_proofs.into(),
        ]
        .concat();

        let (result,) = static_call_helper::<verifyMatchCall>(
            storage,
            verifier_address,
            (batch_verification_bundle_ser,),
        )?
        .into();

        assert_result!(result, VERIFICATION_FAILED_ERROR_MESSAGE)
    }

    /// Nullifies the old wallet and commits to the new wallet
    pub fn rotate_wallet<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        old_wallet_nullifier: ScalarField,
        merkle_root: ScalarField,
        new_wallet_private_shares_commitment: ScalarField,
        new_wallet_public_shares: &[ScalarField],
    ) -> Result<(), Vec<u8>> {
        DarkpoolCoreContract::check_wallet_rotation(
            storage,
            old_wallet_nullifier,
            merkle_root,
            new_wallet_public_shares,
        )?;
        DarkpoolCoreContract::insert_wallet_commitment_to_merkle_tree(
            storage,
            new_wallet_private_shares_commitment,
            new_wallet_public_shares,
        )
    }

    /// Nullifies the old wallet and commits to the new wallet,
    /// verifying a signature over the commitment to the new wallet
    pub fn rotate_wallet_with_signature<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        old_wallet_nullifier: ScalarField,
        merkle_root: ScalarField,
        new_wallet_private_shares_commitment: ScalarField,
        new_wallet_public_shares: &[ScalarField],
        new_wallet_commitment_signature: Vec<u8>,
        old_pk_root: PublicSigningKey,
    ) -> Result<(), Vec<u8>> {
        DarkpoolCoreContract::check_wallet_rotation(
            storage,
            old_wallet_nullifier,
            merkle_root,
            new_wallet_public_shares,
        )?;
        DarkpoolCoreContract::insert_signed_wallet_commitment_to_merkle_tree(
            storage,
            new_wallet_private_shares_commitment,
            new_wallet_public_shares,
            new_wallet_commitment_signature,
            &old_pk_root,
        )
    }

    /// Attempts to nullify the old wallet and ensures that the given Merkle
    /// root is a valid historical root. Logs the wallet udpate if successful.
    pub fn check_wallet_rotation<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        old_wallet_nullifier: ScalarField,
        merkle_root: ScalarField,
        new_wallet_public_shares: &[ScalarField],
    ) -> Result<(), Vec<u8>> {
        DarkpoolCoreContract::check_root_and_nullify(storage, old_wallet_nullifier, merkle_root)?;
        DarkpoolCoreContract::log_wallet_update(new_wallet_public_shares);

        Ok(())
    }

    /// Checks that the given Merkle root is a valid historical root,
    /// and marks the nullifier as spent.
    pub fn check_root_and_nullify<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        nullifier: ScalarField,
        merkle_root: ScalarField,
    ) -> Result<(), Vec<u8>> {
        if_verifying!({
            DarkpoolCoreContract::check_root_in_history(storage, merkle_root)?;
        });

        DarkpoolCoreContract::mark_nullifier_spent(storage, nullifier)
    }

    /// Commits the given note commitment in the Merkle tree
    pub fn commit_note<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        note_commitment: ScalarField,
    ) -> Result<(), Vec<u8>> {
        let note_commitment_u256 = scalar_to_u256(note_commitment);
        let merkle_address = storage.borrow_mut().merkle_address.get();
        delegate_call_helper::<insertNoteCommitmentCall>(
            storage,
            merkle_address,
            (note_commitment_u256,),
        )?;

        evm::log(NotePosted {
            note_commitment: note_commitment_u256,
        });

        Ok(())
    }

    // -----------
    // | LOGGING |
    // -----------

    /// Emits a `WalletUpdated` event with the wallet's public blinder share
    pub fn log_wallet_update(public_wallet_shares: &[ScalarField]) {
        // We assume the wallet blinder is the last scalar serialized into the wallet shares.
        // Unwrapping here is safe because we know the wallet shares are non-empty.
        let wallet_blinder_share = scalar_to_u256(*public_wallet_shares.last().unwrap());
        evm::log(WalletUpdated {
            wallet_blinder_share,
        });
    }
}
