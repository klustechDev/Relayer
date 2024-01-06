//! Miscellaneous helper functions for the contracts.

use alloc::vec::Vec;
use alloy_sol_types::{SolCall, SolType};
use ark_ff::PrimeField;
use contracts_common::{
    custom_serde::{bigint_from_le_bytes, BytesSerializable, ScalarSerializable, SerdeError, pk_to_scalars},
    types::{PublicInputs, ScalarField, PublicSigningKey}, constants::NUM_SCALARS_PK,
};
use stylus_sdk::{
    alloy_primitives::{Address, U256},
    call::{delegate_call, static_call},
    storage::TopLevelStorage,
};

/// Serializes the given statement into scalars, and then into bytes,
/// as expected by the verifier contract.
#[cfg_attr(
    not(any(feature = "darkpool", feature = "darkpool-test-contract")),
    allow(dead_code)
)]
pub fn serialize_statement_for_verification<S: ScalarSerializable>(
    statement: &S,
) -> postcard::Result<Vec<u8>> {
    postcard::to_allocvec(&PublicInputs(statement.serialize_to_scalars().unwrap()))
}

/// Performs a `delegatecall` to the given address, calling the function
/// defined as a `SolCall` with the given arguments.
#[cfg_attr(
    not(any(feature = "darkpool", feature = "darkpool-test-contract")),
    allow(dead_code)
)]
pub fn delegate_call_helper<C: SolCall>(
    storage: &mut impl TopLevelStorage,
    address: Address,
    args: <C::Arguments<'_> as SolType>::RustType,
) -> C::Return {
    let calldata = C::new(args).encode();
    let res = unsafe { delegate_call(storage, address, &calldata).unwrap() };
    C::decode_returns(&res, true /* validate */).unwrap()
}

/// Performs a `staticcall` to the given address, calling the function
/// defined as a `SolCall` with the given arguments.
#[cfg_attr(
    not(any(feature = "darkpool", feature = "darkpool-test-contract")),
    allow(dead_code)
)]
pub fn static_call_helper<C: SolCall>(
    storage: &mut impl TopLevelStorage,
    address: Address,
    args: <C::Arguments<'_> as SolType>::RustType,
) -> C::Return {
    let calldata = C::new(args).encode();
    let res = static_call(storage, address, &calldata).unwrap();
    C::decode_returns(&res, true /* validate */).unwrap()
}

/// Converts a scalar to a U256
#[cfg_attr(
    not(any(
        feature = "darkpool",
        feature = "darkpool-test-contract",
        feature = "merkle",
        feature = "merkle-test-contract"
    )),
    allow(dead_code)
)]
pub fn scalar_to_u256(scalar: ScalarField) -> U256 {
    U256::from_be_slice(&scalar.serialize_to_bytes())
}

/// Converts a U256 to a scalar
#[cfg_attr(
    not(any(
        feature = "darkpool-test-contract",
        feature = "merkle",
        feature = "merkle-test-contract"
    )),
    allow(dead_code)
)]
pub fn u256_to_scalar(u256: U256) -> Result<ScalarField, SerdeError> {
    let bigint = bigint_from_le_bytes(&u256.to_le_bytes_vec())?;
    ScalarField::from_bigint(bigint).ok_or(SerdeError::ScalarConversion)
}

#[cfg_attr(
    not(any(feature = "darkpool", feature = "darkpool-test-contract")),
    allow(dead_code)
)]
pub fn pk_to_u256s(pk: &PublicSigningKey) -> [U256; NUM_SCALARS_PK] {
    let scalars = pk_to_scalars(pk);
    scalars
        .into_iter()
        .map(scalar_to_u256)
        .collect::<Vec<_>>()
        .try_into()
        .unwrap()
}

#[macro_export]
macro_rules! if_verifying {
    ($($logic:tt)*) => {
        #[cfg(not(feature = "no-verify"))]
        {
            $($logic)*
        }
    };
}
