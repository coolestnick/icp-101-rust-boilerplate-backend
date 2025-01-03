#[macro_use]
extern crate serde;
use candid::{CandidType, Decode, Encode};
use ic_cdk::api::time;
use ic_stable_structures::memory_manager::{MemoryId, MemoryManager, VirtualMemory};
use ic_stable_structures::{BoundedStorable, Cell, DefaultMemoryImpl, StableBTreeMap, Storable};
use std::{borrow::Cow, cell::RefCell};

// Type definitions for memory and ID cell
type Memory = VirtualMemory<DefaultMemoryImpl>;
type IdCell = Cell<u64, Memory>;

// Vote struct to represent a vote in the system
#[derive(candid::CandidType, Clone, Serialize, Deserialize, Default)]
struct Vote {
    id: u64,
    voter_id: String,  // Encrypted or hashed voter ID
    candidate: String,
    timestamp: u64,
    proof: String,     // Zero-Knowledge Proof (ZKP) for the vote's validity
}

// Implementing traits for stable storage of Vote struct
impl Storable for Vote {
    fn to_bytes(&self) -> Cow<[u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(bytes.as_ref(), Self).unwrap()
    }
}

impl BoundedStorable for Vote {
    const MAX_SIZE: u32 = 1024;
    const IS_FIXED_SIZE: bool = false;
}

// Memory and storage management
thread_local! {
    static MEMORY_MANAGER: RefCell<MemoryManager<DefaultMemoryImpl>> = RefCell::new(
        MemoryManager::init(DefaultMemoryImpl::default())
    );

    static ID_COUNTER: RefCell<IdCell> = RefCell::new(
        IdCell::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(0))), 0)
            .expect("Cannot create a counter")
    );

    static STORAGE: RefCell<StableBTreeMap<u64, Vote, Memory>> =
        RefCell::new(StableBTreeMap::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(1)))
        ));
}

// Payload struct for submitting votes
#[derive(candid::CandidType, Serialize, Deserialize, Default)]
struct VotePayload {
    candidate: String,
    proof: String,  // The Zero-Knowledge Proof (ZKP) for vote validation
}

// Error enum for handling common errors
#[derive(candid::CandidType, Deserialize, Serialize)]
enum Error {
    NotFound { msg: String },
    InvalidInput { msg: String },
    AlreadyExists { msg: String },
    Unauthorized { msg: String },
}

// Add validation for candidate names
fn validate_candidate(candidate: &str) -> Result<(), Error> {
    if candidate.trim().is_empty() {
        return Err(Error::InvalidInput { msg: "Candidate name cannot be empty or whitespace".to_string() });
    }
    Ok(())
}

// Function to validate input for a VotePayload
fn validate_vote_payload(vote_payload: &VotePayload) -> Result<(), Error> {
    if vote_payload.candidate.trim().is_empty() {
        return Err(Error::InvalidInput { msg: "Candidate name cannot be empty or whitespace".to_string() });
    }
    if vote_payload.proof.trim().is_empty() {
        return Err(Error::InvalidInput { msg: "Proof cannot be empty or whitespace".to_string() });
    }
    Ok(())
}

// Function to validate voter ID
fn validate_voter_id(voter_id: &str) -> Result<(), Error> {
    if voter_id.trim().is_empty() {
        return Err(Error::InvalidInput { msg: "Voter ID cannot be empty or whitespace".to_string() });
    }
    Ok(())
}

// Query function to get a vote by its ID
#[ic_cdk::query]
fn get_vote(id: u64) -> Result<Vote, Error> {
    match _get_vote(&id) {
        Some(vote) => Ok(vote),
        None => Err(Error::NotFound {
            msg: format!("Vote with id={} not found", id),
        }),
    }
}

// Function to add a new vote
#[ic_cdk::update]
fn add_vote(vote_payload: VotePayload, voter_id: String) -> Result<Vote, Error> {
    validate_vote_payload(&vote_payload)?;
    validate_voter_id(&voter_id)?;

    let id = ID_COUNTER.with(|counter| {
        let current_value = *counter.borrow().get();
        counter.borrow_mut().set(current_value + 1)
    })
    .expect("Cannot increment id counter");

    let vote = Vote {
        id,
        voter_id: voter_id.clone(),
        candidate: vote_payload.candidate,
        timestamp: time(),
        proof: vote_payload.proof,
    };

    do_insert(&vote);
    Ok(vote)
}

// Function to insert a vote into storage
fn do_insert(vote: &Vote) {
    STORAGE.with(|service| service.borrow_mut().insert(vote.id, vote.clone()));
}

// Function to retrieve a vote by id
fn _get_vote(id: &u64) -> Option<Vote> {
    STORAGE.with(|service| service.borrow().get(id))
}

// Function to get all votes stored in the system
#[ic_cdk::query]
fn get_all_votes() -> Vec<Vote> {
    STORAGE.with(|service| service.borrow().iter().map(|(_, v)| v.clone()).collect())
}

// Function to get the vote count for a specific candidate
#[ic_cdk::query]
fn get_vote_count(candidate: String) -> u64 {
    STORAGE.with(|service| {
        service.borrow().iter().filter(|(_, vote)| vote.candidate == candidate).count()
    }) as u64
}

// Function to modify an existing vote
#[ic_cdk::update]
fn modify_vote(vote_id: u64, new_vote: VotePayload) -> Result<Vote, Error> {
    validate_vote_payload(&new_vote)?;

    match _get_vote(&vote_id) {
        Some(mut vote) => {
            vote.candidate = new_vote.candidate;
            vote.proof = new_vote.proof;
            vote.timestamp = time(); // Update timestamp for the modified vote
            do_insert(&vote);
            Ok(vote)
        }
        None => Err(Error::NotFound {
            msg: format!("Vote with id={} not found for modification", vote_id),
        }),
    }
}

// Function to get a specific voter's vote by their encrypted voter ID
#[ic_cdk::query]
fn get_voters_vote(voter_id: String) -> Result<Vote, Error> {
    validate_voter_id(&voter_id)?;
    STORAGE.with(|service| {
        service
            .borrow()
            .iter()
            .find(|(_, vote)| vote.voter_id == voter_id)
            .map(|(_, v)| v.clone())
    })
    .ok_or_else(|| Error::NotFound {
        msg: format!("No vote found for voter ID {}", voter_id),
    })
}

// Function to check if a vote's Zero-Knowledge Proof (ZKP) is valid
#[ic_cdk::query]
fn check_zkp_validity(vote_id: u64, proof: String) -> Result<bool, Error> {
    match _get_vote(&vote_id) {
        Some(vote) => Ok(vote.proof == proof),
        None => Err(Error::NotFound {
            msg: format!("Vote with id={} not found to check ZKP", vote_id),
        }),
    }
}

// New: Function to delete a vote by its ID
#[ic_cdk::update]
fn delete_vote(vote_id: u64) -> Result<(), Error> {
    if STORAGE.with(|service| service.borrow_mut().remove(&vote_id)).is_some() {
        Ok(())
    } else {
        Err(Error::NotFound {
            msg: format!("Vote with id={} not found for deletion", vote_id),
        })
    }
}

// New: Function to clear all votes (admin use only)
#[ic_cdk::update]
fn clear_all_votes() -> Result<(), Error> {
    STORAGE.with(|service| {
        let memory = MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(1)));
        *service.borrow_mut() = StableBTreeMap::init(memory);
    });
    Ok(())
}

// New: Function to retrieve votes by candidate
#[ic_cdk::query]
fn get_votes_by_candidate(candidate: String) -> Vec<Vote> {
    STORAGE.with(|service| {
        service
            .borrow()
            .iter()
            .filter(|(_, vote)| vote.candidate == candidate)
            .map(|(_, v)| v.clone())
            .collect()
    })
}


// Export Candid interface for the canister
ic_cdk::export_candid!();
