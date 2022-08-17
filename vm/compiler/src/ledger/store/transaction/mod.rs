// Copyright (C) 2019-2022 Aleo Systems Inc.
// This file is part of the snarkVM library.

// The snarkVM library is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// The snarkVM library is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with the snarkVM library. If not, see <https://www.gnu.org/licenses/>.

mod deployment;
pub use deployment::*;

mod execution;
pub use execution::*;

use crate::{
    cow_to_copied,
    ledger::{
        map::{memory_map::MemoryMap, Map, MapRead, OrAbort},
        store::{TransitionMemory, TransitionStorage, TransitionStore},
        AdditionalFee,
        Transaction,
    },
    process::{Deployment, Execution},
    program::Program,
    snark::{Certificate, VerifyingKey},
};
use console::{
    network::prelude::*,
    program::{Identifier, ProgramID},
};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

#[derive(Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransactionType {
    /// A transaction that is a deployment.
    Deploy,
    /// A transaction that is an execution.
    Execute,
}

/// A trait for transaction storage.
pub trait TransactionStorage<N: Network>: Clone + Sync {
    /// The mapping of `transaction ID` to `transaction type`.
    type IDMap: for<'a> Map<'a, N::TransactionID, TransactionType>;
    /// The deployment storage.
    type DeploymentStorage: DeploymentStorage<N, TransitionStorage = Self::TransitionStorage>;
    /// The execution storage.
    type ExecutionStorage: ExecutionStorage<N, TransitionStorage = Self::TransitionStorage>;
    /// The transition storage.
    type TransitionStorage: TransitionStorage<N>;

    /// Initializes the transaction storage.
    fn open(transition_store: TransitionStore<N, Self::TransitionStorage>) -> Result<Self>;

    /// Returns the ID map.
    fn id_map(&self) -> &Self::IDMap;
    /// Returns the deployment store.
    fn deployment_store(&self) -> &DeploymentStore<N, Self::DeploymentStorage>;
    /// Returns the execution store.
    fn execution_store(&self) -> &ExecutionStore<N, Self::ExecutionStorage>;

    /// Starts an atomic batch write operation.
    fn start_atomic(&self) {
        self.id_map().start_atomic();
        self.deployment_store().start_atomic();
        self.execution_store().start_atomic();
    }

    /// Aborts an atomic batch write operation.
    fn abort_atomic(&self) {
        self.id_map().abort_atomic();
        self.deployment_store().abort_atomic();
        self.execution_store().abort_atomic();
    }

    /// Finishes an atomic batch write operation.
    fn finish_atomic(&self) {
        self.id_map().finish_atomic();
        self.deployment_store().finish_atomic();
        self.execution_store().finish_atomic();
    }

    /// Stores the given `transaction` into storage.
    fn insert(&self, transaction: &Transaction<N>) -> Result<()> {
        // Start an atomic batch write operation.
        self.start_atomic();

        match transaction {
            Transaction::Deploy(..) => {
                // Store the transaction type.
                self.id_map().insert(transaction.id(), TransactionType::Deploy).or_abort(|| self.abort_atomic())?;
                // Store the deployment transaction.
                self.deployment_store().insert(transaction).or_abort(|| self.abort_atomic())?;
            }
            Transaction::Execute(..) => {
                // Store the transaction type.
                self.id_map().insert(transaction.id(), TransactionType::Execute).or_abort(|| self.abort_atomic())?;
                // Store the execution transaction.
                self.execution_store().insert(transaction).or_abort(|| self.abort_atomic())?;
            }
        }

        // Finish an atomic batch write operation.
        self.finish_atomic();

        Ok(())
    }

    /// Removes the transaction for the given `transaction ID`.
    fn remove(&self, transaction_id: &N::TransactionID) -> Result<()> {
        // Retrieve the transaction type.
        let transaction_type = match self.id_map().get(transaction_id)? {
            Some(transaction_type) => cow_to_copied!(transaction_type),
            None => bail!("Failed to get the type for transaction '{transaction_id}'"),
        };

        // Start an atomic batch write operation.
        self.start_atomic();

        // Remove the transaction type.
        self.id_map().remove(transaction_id).or_abort(|| self.abort_atomic())?;
        // Remove the transaction.
        match transaction_type {
            // Remove the deployment transaction.
            TransactionType::Deploy => {
                self.deployment_store().remove(transaction_id).or_abort(|| self.abort_atomic())?
            }
            // Remove the execution transaction.
            TransactionType::Execute => {
                self.execution_store().remove(transaction_id).or_abort(|| self.abort_atomic())?
            }
        };

        // Finish an atomic batch write operation.
        self.finish_atomic();

        Ok(())
    }

    /// Returns the transaction ID that contains the given `transition ID`.
    fn find_transaction_id(&self, transition_id: &N::TransitionID) -> Result<Option<N::TransactionID>> {
        self.execution_store().find_transaction_id(transition_id)
    }

    /// Returns the transaction ID that contains the given `program ID`.
    fn find_deployment_id(&self, program_id: &ProgramID<N>) -> Result<Option<N::TransactionID>> {
        self.deployment_store().find_transaction_id(program_id)
    }

    /// Returns the transaction for the given `transaction ID`.
    fn get_transaction(&self, transaction_id: &N::TransactionID) -> Result<Option<Transaction<N>>> {
        // Retrieve the transaction type.
        let transaction_type = match self.id_map().get(transaction_id)? {
            Some(transaction_type) => cow_to_copied!(transaction_type),
            None => return Ok(None),
        };
        // Retrieve the transaction.
        match transaction_type {
            // Return the deployment transaction.
            TransactionType::Deploy => self.deployment_store().get_transaction(transaction_id),
            // Return the execution transaction.
            TransactionType::Execute => self.execution_store().get_transaction(transaction_id),
        }
    }
}

/// An in-memory transaction storage.
#[derive(Clone)]
pub struct TransactionMemory<N: Network> {
    /// The mapping of `transaction ID` to `transaction type`.
    id_map: MemoryMap<N::TransactionID, TransactionType>,
    /// The deployment store.
    deployment_store: DeploymentStore<N, DeploymentMemory<N>>,
    /// The execution store.
    execution_store: ExecutionStore<N, ExecutionMemory<N>>,
}

#[rustfmt::skip]
impl<N: Network> TransactionStorage<N> for TransactionMemory<N> {
    type IDMap = MemoryMap<N::TransactionID, TransactionType>;
    type DeploymentStorage = DeploymentMemory<N>;
    type ExecutionStorage = ExecutionMemory<N>;
    type TransitionStorage = TransitionMemory<N>;

    /// Initializes the transaction storage.
    fn open(transition_store: TransitionStore<N, Self::TransitionStorage>) -> Result<Self> {
        // Initialize the deployment store.
        let deployment_store = DeploymentStore::<N, DeploymentMemory<N>>::open(transition_store.clone())?;
        // Initialize the execution store.
        let execution_store = ExecutionStore::<N, ExecutionMemory<N>>::open(transition_store)?;
        // Return the transaction storage.
        Ok(Self { id_map: MemoryMap::default(), deployment_store, execution_store })
    }

    /// Returns the ID map.
    fn id_map(&self) -> &Self::IDMap {
        &self.id_map
    }

    /// Returns the deployment store.
    fn deployment_store(&self) -> &DeploymentStore<N, Self::DeploymentStorage> {
        &self.deployment_store
    }

    /// Returns the execution store.
    fn execution_store(&self) -> &ExecutionStore<N, Self::ExecutionStorage> {
        &self.execution_store
    }
}

/// The transaction store.
#[derive(Clone)]
pub struct TransactionStore<N: Network, T: TransactionStorage<N>> {
    /// The map of `transaction ID` to `transaction type`.
    transaction_ids: T::IDMap,
    /// The transaction storage.
    storage: T,
}

impl<N: Network, T: TransactionStorage<N>> TransactionStore<N, T> {
    /// Initializes the transaction store.
    pub fn open(transition_store: TransitionStore<N, T::TransitionStorage>) -> Result<Self> {
        // Initialize the transaction storage.
        let storage = T::open(transition_store)?;
        // Return the transaction store.
        Ok(Self { transaction_ids: storage.id_map().clone(), storage })
    }

    /// Initializes a transaction store from storage.
    pub fn from(storage: T) -> Self {
        Self { transaction_ids: storage.id_map().clone(), storage }
    }

    /// Stores the given `transaction` into storage.
    pub fn insert(&self, transaction: &Transaction<N>) -> Result<()> {
        self.storage.insert(transaction)
    }

    /// Removes the transaction for the given `transaction ID`.
    pub fn remove(&self, transaction_id: &N::TransactionID) -> Result<()> {
        self.storage.remove(transaction_id)
    }

    /// Returns the transition store.
    pub fn transition_store(&self) -> &TransitionStore<N, T::TransitionStorage> {
        self.storage.execution_store().transition_store()
    }

    /// Starts an atomic batch write operation.
    pub fn start_atomic(&self) {
        self.storage.start_atomic();
    }

    /// Aborts an atomic batch write operation.
    pub fn abort_atomic(&self) {
        self.storage.abort_atomic();
    }

    /// Finishes an atomic batch write operation.
    pub fn finish_atomic(&self) {
        self.storage.finish_atomic();
    }
}

impl<N: Network, T: TransactionStorage<N>> TransactionStore<N, T> {
    /// Returns the transaction for the given `transaction ID`.
    pub fn get_transaction(&self, transaction_id: &N::TransactionID) -> Result<Option<Transaction<N>>> {
        self.storage.get_transaction(transaction_id)
    }

    /// Returns the deployment for the given `transaction ID`.
    pub fn get_deployment(&self, transaction_id: &N::TransactionID) -> Result<Option<Deployment<N>>> {
        // Retrieve the transaction type.
        let transaction_type = match self.transaction_ids.get(transaction_id)? {
            Some(transaction_type) => cow_to_copied!(transaction_type),
            None => bail!("Failed to get the type for transaction '{transaction_id}'"),
        };
        // Retrieve the deployment.
        match transaction_type {
            // Return the deployment.
            TransactionType::Deploy => self.storage.deployment_store().get_deployment(transaction_id),
            // Throw an error.
            TransactionType::Execute => bail!("Tried to get a deployment for execution transaction '{transaction_id}'"),
        }
    }

    /// Returns the execution for the given `transaction ID`.
    pub fn get_execution(&self, transaction_id: &N::TransactionID) -> Result<Option<Execution<N>>> {
        // Retrieve the transaction type.
        let transaction_type = match self.transaction_ids.get(transaction_id)? {
            Some(transaction_type) => cow_to_copied!(transaction_type),
            None => bail!("Failed to get the type for transaction '{transaction_id}'"),
        };
        // Retrieve the execution.
        match transaction_type {
            // Throw an error.
            TransactionType::Deploy => bail!("Tried to get an execution for deployment transaction '{transaction_id}'"),
            // Return the execution.
            TransactionType::Execute => self.storage.execution_store().get_execution(transaction_id),
        }
    }

    /// Returns the edition for the given `transaction ID`.
    pub fn get_edition(&self, transaction_id: &N::TransactionID) -> Result<Option<u16>> {
        // Retrieve the transaction type.
        let transaction_type = match self.transaction_ids.get(transaction_id)? {
            Some(transaction_type) => cow_to_copied!(transaction_type),
            None => bail!("Failed to get the type for transaction '{transaction_id}'"),
        };
        // Retrieve the edition.
        match transaction_type {
            TransactionType::Deploy => {
                // Retrieve the program ID.
                let program_id = self.storage.deployment_store().get_program_id(transaction_id)?;
                // Return the edition.
                match program_id {
                    Some(program_id) => self.storage.deployment_store().get_edition(&program_id),
                    None => bail!("Failed to get the program ID for deployment transaction '{transaction_id}'"),
                }
            }
            // Return the edition.
            TransactionType::Execute => self.storage.execution_store().get_edition(transaction_id),
        }
    }

    /// Returns the program ID for the given `transaction ID`.
    pub fn get_program_id(&self, transaction_id: &N::TransactionID) -> Result<Option<ProgramID<N>>> {
        self.storage.deployment_store().get_program_id(transaction_id)
    }

    /// Returns the program for the given `program ID`.
    pub fn get_program(&self, program_id: &ProgramID<N>) -> Result<Option<Program<N>>> {
        self.storage.deployment_store().get_program(program_id)
    }

    /// Returns the verifying key for the given `(program ID, function name)`.
    pub fn get_verifying_key(
        &self,
        program_id: &ProgramID<N>,
        function_name: &Identifier<N>,
    ) -> Result<Option<VerifyingKey<N>>> {
        self.storage.deployment_store().get_verifying_key(program_id, function_name)
    }

    /// Returns the certificate for the given `(program ID, function name)`.
    pub fn get_certificate(
        &self,
        program_id: &ProgramID<N>,
        function_name: &Identifier<N>,
    ) -> Result<Option<Certificate<N>>> {
        self.storage.deployment_store().get_certificate(program_id, function_name)
    }

    /// Returns the additional fee for the given `transaction ID`.
    pub fn get_additional_fee(&self, transaction_id: &N::TransactionID) -> Result<Option<AdditionalFee<N>>> {
        // Retrieve the transaction type.
        let transaction_type = match self.transaction_ids.get(transaction_id)? {
            Some(transaction_type) => cow_to_copied!(transaction_type),
            None => bail!("Failed to get the type for transaction '{transaction_id}'"),
        };
        // Retrieve the fee.
        match transaction_type {
            // Return the fee.
            TransactionType::Deploy => self.storage.deployment_store().get_additional_fee(transaction_id),
            // Return the fee.
            TransactionType::Execute => self.storage.execution_store().get_additional_fee(transaction_id),
        }
    }
}

impl<N: Network, T: TransactionStorage<N>> TransactionStore<N, T> {
    /// Returns the transaction ID that contains the given `transition ID`.
    pub fn find_transaction_id(&self, transition_id: &N::TransitionID) -> Result<Option<N::TransactionID>> {
        self.storage.execution_store().find_transaction_id(transition_id)
    }

    /// Returns the transaction ID that contains the given `program ID`.
    pub fn find_deployment_id(&self, program_id: &ProgramID<N>) -> Result<Option<N::TransactionID>> {
        self.storage.deployment_store().find_transaction_id(program_id)
    }
}

impl<N: Network, T: TransactionStorage<N>> TransactionStore<N, T> {
    /// Returns `true` if the given transaction ID exists.
    pub fn contains_transaction_id(&self, transaction_id: &N::TransactionID) -> Result<bool> {
        self.transaction_ids.contains_key(transaction_id)
    }

    /// Returns `true` if the given program ID exists.
    pub fn contains_program_id(&self, program_id: &ProgramID<N>) -> Result<bool> {
        self.storage.deployment_store().contains_program_id(program_id)
    }
}

impl<N: Network, T: TransactionStorage<N>> TransactionStore<N, T> {
    /// Returns an iterator over the transaction IDs, for all transactions.
    pub fn transaction_ids(&self) -> impl '_ + Iterator<Item = Cow<'_, N::TransactionID>> {
        self.transaction_ids.keys()
    }

    /// Returns an iterator over the deployment transaction IDs, for all deployments.
    pub fn deployment_ids(&self) -> impl '_ + Iterator<Item = Cow<'_, N::TransactionID>> {
        self.storage.deployment_store().deployment_ids()
    }

    /// Returns an iterator over the execution transaction IDs, for all executions.
    pub fn execution_ids(&self) -> impl '_ + Iterator<Item = Cow<'_, N::TransactionID>> {
        self.storage.execution_store().execution_ids()
    }

    /// Returns an iterator over the program IDs, for all deployments.
    pub fn program_ids(&self) -> impl '_ + Iterator<Item = Cow<'_, ProgramID<N>>> {
        self.storage.deployment_store().program_ids()
    }

    /// Returns an iterator over the programs, for all deployments.
    pub fn programs(&self) -> impl '_ + Iterator<Item = Cow<'_, Program<N>>> {
        self.storage.deployment_store().programs()
    }

    /// Returns an iterator over the `((program ID, function name, edition), verifying key)`, for all deployments.
    pub fn verifying_keys(
        &self,
    ) -> impl '_ + Iterator<Item = (Cow<'_, (ProgramID<N>, Identifier<N>, u16)>, Cow<'_, VerifyingKey<N>>)> {
        self.storage.deployment_store().verifying_keys()
    }

    /// Returns an iterator over the `((program ID, function name, edition), certificate)`, for all deployments.
    pub fn certificates(
        &self,
    ) -> impl '_ + Iterator<Item = (Cow<'_, (ProgramID<N>, Identifier<N>, u16)>, Cow<'_, Certificate<N>>)> {
        self.storage.deployment_store().certificates()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_get_remove() {
        // Sample the transactions.
        for transaction in [
            crate::ledger::vm::test_helpers::sample_deployment_transaction(),
            crate::ledger::vm::test_helpers::sample_execution_transaction(),
        ] {
            let transaction_id = transaction.id();

            // Initialize a new transition store.
            let transition_store = TransitionStore::<_, TransitionMemory<_>>::open().unwrap();
            // Initialize a new transaction store.
            let transaction_store = TransactionStore::<_, TransactionMemory<_>>::open(transition_store).unwrap();

            // Ensure the transaction does not exist.
            let candidate = transaction_store.get_transaction(&transaction_id).unwrap();
            assert_eq!(None, candidate);

            // Insert the transaction.
            transaction_store.insert(&transaction).unwrap();

            // Retrieve the transaction.
            let candidate = transaction_store.get_transaction(&transaction_id).unwrap();
            assert_eq!(Some(transaction), candidate);

            // Remove the transaction.
            transaction_store.remove(&transaction_id).unwrap();

            // Ensure the transaction does not exist.
            let candidate = transaction_store.get_transaction(&transaction_id).unwrap();
            assert_eq!(None, candidate);
        }
    }

    #[test]
    fn test_find_transaction_id() {
        // Sample the execution transaction.
        let transaction = crate::ledger::vm::test_helpers::sample_execution_transaction();
        let transaction_id = transaction.id();
        let transition_ids = match transaction {
            Transaction::Execute(_, ref execution, _) => {
                execution.clone().into_transitions().map(|transition| *transition.id()).collect::<Vec<_>>()
            }
            _ => panic!("Incorrect transaction type"),
        };

        // Initialize a new transition store.
        let transition_store = TransitionStore::<_, TransitionMemory<_>>::open().unwrap();
        // Initialize a new transaction store.
        let transaction_store = TransactionStore::<_, TransactionMemory<_>>::open(transition_store).unwrap();

        // Ensure the execution transaction does not exist.
        let candidate = transaction_store.get_transaction(&transaction_id).unwrap();
        assert_eq!(None, candidate);

        for transition_id in transition_ids {
            // Ensure the transaction ID is not found.
            let candidate = transaction_store.find_transaction_id(&transition_id).unwrap();
            assert_eq!(None, candidate);

            // Insert the transaction.
            transaction_store.insert(&transaction).unwrap();

            // Find the transaction ID.
            let candidate = transaction_store.find_transaction_id(&transition_id).unwrap();
            assert_eq!(Some(transaction_id), candidate);

            // Remove the transaction.
            transaction_store.remove(&transaction_id).unwrap();

            // Ensure the transaction ID is not found.
            let candidate = transaction_store.find_transaction_id(&transition_id).unwrap();
            assert_eq!(None, candidate);
        }
    }
}
