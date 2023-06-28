// Copyright (C) 2019-2023 Aleo Systems Inc.
// This file is part of the snarkVM library.

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at:
// http://www.apache.org/licenses/LICENSE-2.0

// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::Identifier;
use snarkvm_circuit_network::Aleo;
use snarkvm_circuit_types::{environment::prelude::*, Eject, Inject, Mode, Parser, ParserResult};

use std::{
    fmt,
    fmt::{Debug, Display, Formatter},
    str::FromStr,
};

/// A register `Access`.
#[derive(Clone)]
pub enum Access<A: Aleo> {
    // TODO (d0cd): Add the index variant.
    /// The access is a member.
    Member(Identifier<A>),
}

#[cfg(console)]
impl<A: Aleo> Inject for Access<A> {
    type Primitive = console::Access<A::Network>;

    /// Initializes a new access circuit from a primitive.
    fn new(mode: Mode, plaintext: Self::Primitive) -> Self {
        match plaintext {
            Self::Primitive::Member(identifier) => Self::Member(Identifier::new(mode, identifier)),
        }
    }
}

#[cfg(console)]
impl<A: Aleo> Eject for Access<A> {
    type Primitive = console::Access<A::Network>;

    /// Ejects the mode of the access.
    fn eject_mode(&self) -> Mode {
        match self {
            Self::Member(member) => member.eject_mode(),
        }
    }

    /// Ejects the access.
    fn eject_value(&self) -> Self::Primitive {
        match self {
            Self::Member(identifier) => console::Access::Member(identifier.eject_value()),
        }
    }
}

#[cfg(console)]
impl<A: Aleo> Parser for Access<A> {
    /// Parses a UTF-8 string into an access.
    #[inline]
    fn parse(string: &str) -> ParserResult<Self> {
        // Parse the identifier from the string.
        let (string, access) = console::Access::parse(string)?;

        Ok((string, Access::constant(access)))
    }
}

#[cfg(console)]
impl<A: Aleo> FromStr for Access<A> {
    type Err = Error;

    /// Parses a UTF-8 string into an identifier.
    #[inline]
    fn from_str(string: &str) -> Result<Self> {
        match Self::parse(string) {
            Ok((remainder, object)) => {
                // Ensure the remainder is empty.
                ensure!(remainder.is_empty(), "Failed to parse string. Found invalid character in: \"{remainder}\"");
                // Return the object.
                Ok(object)
            }
            Err(error) => bail!("Failed to parse string. {error}"),
        }
    }
}

#[cfg(console)]
impl<A: Aleo> Debug for Access<A> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

#[cfg(console)]
impl<A: Aleo> Display for Access<A> {
    /// Prints the identifier as a string.
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{}", self.eject_value())
    }
}

impl<A: Aleo> Eq for Access<A> {}

impl<A: Aleo> PartialEq for Access<A> {
    /// Implements the `Eq` trait for the access.
    fn eq(&self, other: &Self) -> bool {
        self.eject_value() == other.eject_value()
    }
}
