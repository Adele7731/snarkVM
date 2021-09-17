// Copyright (C) 2019-2021 Aleo Systems Inc.
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

use crate::{AccountError, AccountScheme, Address, Network, PrivateKey, ViewKey};

use rand::{CryptoRng, Rng};
use std::{
    convert::{TryFrom, TryInto},
    fmt,
};

#[derive(Derivative)]
#[derivative(Clone(bound = "N: Network"))]
pub struct Account<N: Network> {
    private_key: PrivateKey<N>,
    pub view_key: ViewKey<N>,
    pub address: Address<N>,
}

impl<N: Network> AccountScheme for Account<N> {
    type Address = Address<N>;
    type PrivateKey = PrivateKey<N>;
    type ViewKey = ViewKey<N>;

    /// Creates a new account.
    fn new<R: Rng + CryptoRng>(rng: &mut R) -> Result<Self, AccountError> {
        PrivateKey::new(rng).try_into()
    }

    /// Returns a reference to the private key.
    fn private_key(&self) -> &Self::PrivateKey {
        &self.private_key
    }
}

impl<N: Network> TryFrom<PrivateKey<N>> for Account<N> {
    type Error = AccountError;

    /// Creates an account from a private key.
    fn try_from(private_key: PrivateKey<N>) -> Result<Self, Self::Error> {
        let view_key = ViewKey::try_from(&private_key)?;
        let address = Address::try_from(&private_key)?;

        Ok(Self {
            private_key,
            view_key,
            address,
        })
    }
}

impl<N: Network> fmt::Display for Account<N> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Account {{ private_key: {}, view_key: {}, address: {} }}",
            self.private_key, self.view_key, self.address,
        )
    }
}

impl<N: Network> fmt::Debug for Account<N> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Account {{ private_key: {:?}, view_key: {:?}, address: {:?} }}",
            self.private_key, self.view_key, self.address,
        )
    }
}
