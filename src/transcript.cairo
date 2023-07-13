//! A simple Fiat-Shamir transcript that uses a Keccak256 hash chain.

use traits::Into;
use array::ArrayTrait;
use keccak::keccak_u256s_le_inputs;
use ec::{ec_point_unwrap, ec_point_non_zero};

use renegade_contracts::utils::{math::hash_to_felt, constants::SHIFT_128};


const TRANSCRIPT_SEED: felt252 = 'merlin seed';

#[derive(Drop)]
struct Transcript {
    /// The current state of the hash chain.
    state: u256,
}

#[generate_trait]
impl TranscriptImpl of TranscriptTrait {
    fn new(label: felt252) -> Transcript {
        let mut data = ArrayTrait::new();
        data.append(label.into());
        let state = keccak_u256s_le_inputs(data.span());
        Transcript { state }
    }

    /// Absorb an arbitrary-length message into the transcript,
    /// hashing it together with the label & the current state.
    // TODO: Could make this an Array of u64s... see what feels better
    fn append_message(ref self: Transcript, label: felt252, mut message: Array<u256>) {
        message.append(label.into());
        message.append(self.state);
        self.state = keccak_u256s_le_inputs(message.span());
    }

    /// Absorb a u64 into the transcript
    fn append_u64(ref self: Transcript, label: felt252, x: u64) {
        let mut message = ArrayTrait::new();
        message.append(x.into());
        self.append_message(label, message);
    }

    /// Squeeze a challenge u256 out of the transcript.
    fn challenge_u256(ref self: Transcript, label: felt252) -> u256 {
        let mut data = ArrayTrait::new();
        data.append(label.into());
        data.append(self.state);
        self.state = keccak_u256s_le_inputs(data.span());

        let mut data = ArrayTrait::new();
        data.append(self.state);
        keccak_u256s_le_inputs(data.span())
    }
}

#[generate_trait]
impl TranscriptProtocolImpl of TranscriptProtocol {
    /// Append a domain separator for an `n`-bit, `m`-party range proof.
    fn rangeproof_domain_sep(ref self: Transcript, n: u64, m: u64) {
        self.append_dom_sep('rangeproof v1');
        self.append_u64('n', n);
        self.append_u64('m', m);
    }

    /// Append a domain separator for a length-`n` inner product proof.
    fn innerproduct_domain_sep(ref self: Transcript, n: u64) {
        self.append_dom_sep('ipp v1');
        self.append_u64('n', n);
    }

    /// Append a domain separator for a constraint system.
    fn r1cs_domain_sep(ref self: Transcript) {
        self.append_dom_sep('r1cs v1');
    }

    /// Commit a domain separator for a CS without randomized constraints.
    fn r1cs_1phase_domain_sep(ref self: Transcript) {
        self.append_dom_sep('r1cs-1phase');
    }

    /// Append a `scalar` with the given `label`.
    fn append_scalar(ref self: Transcript, label: felt252, scalar: felt252) {
        let mut message = ArrayTrait::new();
        message.append(scalar.into());
        self.append_message(label, message);
    }

    /// Append a `point` with the given `label`.
    /// Panics if the point is the identity.
    fn validate_and_append_point(ref self: Transcript, label: felt252, point: EcPoint, ) {
        let mut message = ArrayTrait::new();
        let (x, y) = ec_point_unwrap(ec_point_non_zero(point));
        message.append(x.into());
        message.append(y.into());
        self.append_message(label, message);
    }

    /// Compute a `label`ed challenge variable.
    fn challenge_scalar(ref self: Transcript, label: felt252) -> felt252 {
        hash_to_felt(self.challenge_u256(label))
    }

    // -----------
    // | HELPERS |
    // -----------

    /// Append a domain separator to the transcript.
    fn append_dom_sep(ref self: Transcript, dom_sep: u256) {
        let mut message = ArrayTrait::new();
        message.append(dom_sep);
        self.append_message('dom-sep', message);
    }
}
