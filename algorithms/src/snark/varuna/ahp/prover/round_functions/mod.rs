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

use crate::{
    fft::{DensePolynomial, EvaluationDomain},
    r1cs::ConstraintSynthesizer,
    snark::varuna::{
        ahp::{indexer::Circuit, AHPError, AHPForR1CS},
        prover,
        SNARKMode,
    },
};
use snarkvm_fields::PrimeField;

use anyhow::Result;
use itertools::Itertools;
use rand::Rng;
use rand_core::CryptoRng;
use std::collections::BTreeMap;

#[cfg(not(feature = "std"))]
use snarkvm_utilities::println;
use snarkvm_utilities::{cfg_iter, cfg_iter_mut};

#[cfg(not(feature = "serial"))]
use rayon::prelude::*;

mod fifth;
mod first;
mod fourth;
mod second;
mod third;

impl<F: PrimeField, MM: SNARKMode> AHPForR1CS<F, MM> {
    /// Initialize the AHP prover.
    pub fn init_prover<'a, C: ConstraintSynthesizer<F>, R: Rng + CryptoRng>(
        circuits_to_constraints: &BTreeMap<&'a Circuit<F, MM>, &[C]>,
        rng: &mut R,
    ) -> Result<prover::State<'a, F, MM>, AHPError> {
        let init_time = start_timer!(|| "AHP::Prover::Init");

        let mut randomizing_assignments = Vec::with_capacity(circuits_to_constraints.len());
        for constraints in circuits_to_constraints.values() {
            let mut circuit_assignments = Vec::with_capacity(constraints.len());
            for _ in 0..constraints.len() {
                if MM::ZK {
                    let a = F::rand(rng);
                    let b = F::rand(rng);
                    let c = a * b;
                    circuit_assignments.push(Some([a, b, c]));
                } else {
                    circuit_assignments.push(None);
                }
            }
            randomizing_assignments.push(circuit_assignments);
        }

        let indices_and_assignments = circuits_to_constraints
            .iter()
            .zip_eq(randomizing_assignments.into_iter())
            .map(|((circuit, constraints), circuit_rand_assignments)| {
                let num_non_zero_a = circuit.index_info.num_non_zero_a;
                let num_non_zero_b = circuit.index_info.num_non_zero_b;
                let num_non_zero_c = circuit.index_info.num_non_zero_c;

                let assignments = cfg_iter!(constraints)
                    .zip(circuit_rand_assignments)
                    .enumerate()
                    .map(|(_i, (instance, rand_assignments))| {
                        let constraint_time = start_timer!(|| format!(
                            "Generating constraints and witnesses for {:?} and index {_i}",
                            circuit.id
                        ));
                        let mut pcs = prover::ConstraintSystem::new();
                        instance.generate_constraints(&mut pcs)?;
                        end_timer!(constraint_time);

                        let padding_time =
                            start_timer!(|| format!("Padding matrices for {:?} and index {_i}", circuit.id));

                        MM::ZK.then(|| {
                            crate::snark::varuna::ahp::matrices::add_randomizing_variables::<_, _>(
                                &mut pcs,
                                rand_assignments,
                            )
                        });
                        crate::snark::varuna::ahp::matrices::pad_input_for_indexer_and_prover(&mut pcs);

                        end_timer!(padding_time);

                        let prover::ConstraintSystem {
                            public_variables: padded_public_variables,
                            private_variables,
                            num_constraints,
                            num_public_variables,
                            num_private_variables,
                            ..
                        } = pcs;

                        assert_eq!(padded_public_variables.len(), num_public_variables);
                        assert!(padded_public_variables[0].is_one());
                        assert_eq!(private_variables.len(), num_private_variables);

                        if cfg!(debug_assertions) {
                            println!("Number of padded public variables in Prover::Init: {num_public_variables}");
                            println!("Number of private variables: {num_private_variables}");
                            println!("Number of constraints: {num_constraints}");
                            println!("Number of non-zero entries in A: {num_non_zero_a}");
                            println!("Number of non-zero entries in B: {num_non_zero_b}");
                            println!("Number of non-zero entries in C: {num_non_zero_c}");
                        }

                        if circuit.index_info.num_constraints != num_constraints
                            || circuit.index_info.num_variables != (num_public_variables + num_private_variables)
                        {
                            return Err(AHPError::InstanceDoesNotMatchIndex);
                        }

                        Self::formatted_public_input_is_admissible(&padded_public_variables)?;

                        let eval_z_a_time = start_timer!(|| format!("For {:?}, evaluating z_A_{_i}", circuit.id));
                        let z_a = cfg_iter!(circuit.a)
                            .map(|row| {
                                inner_product(&padded_public_variables, &private_variables, row, num_public_variables)
                            })
                            .collect();
                        end_timer!(eval_z_a_time);

                        let eval_z_b_time = start_timer!(|| format!("For {:?}, evaluating z_B_{_i}", circuit.id));
                        let z_b = cfg_iter!(circuit.b)
                            .map(|row| {
                                inner_product(&padded_public_variables, &private_variables, row, num_public_variables)
                            })
                            .collect();
                        end_timer!(eval_z_b_time);

                        let eval_z_c_time = start_timer!(|| format!("For {:?}, evaluating z_C_{_i}", circuit.id));
                        let z_c = cfg_iter!(circuit.c)
                            .map(|row| {
                                inner_product(&padded_public_variables, &private_variables, row, num_public_variables)
                            })
                            .collect();
                        end_timer!(eval_z_c_time);

                        end_timer!(init_time);
                        Ok(prover::Assignments::<F>(padded_public_variables, private_variables, z_a, z_b, z_c))
                    })
                    .collect::<Result<Vec<prover::Assignments<F>>, AHPError>>()?;
                Ok((*circuit, assignments))
            })
            .collect::<Result<BTreeMap<&'a Circuit<F, MM>, Vec<prover::Assignments<F>>>, AHPError>>()?;

        let state = prover::State::initialize(indices_and_assignments)?;

        Ok(state)
    }

    /// Throughout the protocol, we are tasked with computing a zerocheck or sumcheck
    /// of multiple polynomials over different domains.
    /// These can be combined into a single check by taking a random linear combination
    /// of the polynomials and multiplying them by an appropriate selector polynomial.
    /// This function applies the random combiner and selector in an optimized way
    fn apply_randomized_selector(
        poly: &mut DensePolynomial<F>,
        combiner: F,
        target_domain: &EvaluationDomain<F>,
        src_domain: &EvaluationDomain<F>,
        remainder_witness: bool,
    ) -> Result<(DensePolynomial<F>, Option<DensePolynomial<F>>)> {
        // Let H = target_domain;
        // Let H_i = src_domain;
        // Let v_H := H.vanishing_polynomial();
        // Let v_H_i := H_i.vanishing_polynomial();
        // Let s_i := H.selector_polynomial(H_i) = (v_H / v_H_i) * (H_i.size() / H.size());
        // Let c_i := circuit combiner
        // Let poly_i := circuit specific polynomial which is being checked

        // Instead of just multiplying each poly_i by `s_i*c_i`, we reorder the check to cancel out division by v_H
        // This removes a mul and div by v_H operation over each circuit's (target_domain - src_domain)
        // We have two scenario's: either we return a remainder witness or there is none.
        if !remainder_witness {
            // Substituting in s_i, we get that poly_i * s_i / v_H = poly_i / v_H * (H_i.size() / H.size());
            let selector_time = start_timer!(|| "Compute selector without remainder witness");
            let (mut h_i, remainder) = poly
                .divide_by_vanishing_poly(*src_domain)
                .ok_or(anyhow::anyhow!("could not divide by vanishing poly"))?;
            assert!(remainder.is_zero());
            let multiplier = combiner * src_domain.size_as_field_element * target_domain.size_inv;
            cfg_iter_mut!(h_i.coeffs).for_each(|c| *c *= multiplier);
            end_timer!(selector_time);
            Ok((h_i, None))
        } else {
            // Substituting in s_i, we get that:
            // \sum_i{poly_i}/v_H = \sum{h_i*v_H + x_g_i}
            // \sum_i{c_i*s_i*(poly_i/v_H - x_g_i)} = \sum{h_i*v_H}
            // \sum_i{c_i*(H_i.size()/H.size())*(poly_i/v_H_i - x_g_i*v_H/v_H_i)} = \sum{h_i*v_H}
            // \sum_i{c_i*(H_i.size()/H.size())*(poly_i/v_H_i} = \sum{h_i*v_H} + \sum{c_i*x_g_i*(v_H/v_H_i)*(H_i.size()/H.size())}
            // (\sum_i{c_i*s_i*poly_i})/v_H = \sum{h_i*v_H} + \sum{c_i*s_i*x_g_i}
            // (\sum_i{c_i*s_i*poly_i})/v_H = h_1*v_H + x_g_1
            // That's what we're computing here.
            let selector_time = start_timer!(|| "Compute selector with remainder witness");
            let multiplier = combiner * src_domain.size_as_field_element * target_domain.size_inv;
            cfg_iter_mut!(poly.coeffs).for_each(|c| *c *= multiplier);
            let (h_i, mut xg_i) = poly.divide_by_vanishing_poly(*src_domain).unwrap();
            xg_i = xg_i.mul_by_vanishing_poly(*target_domain);
            let (xg_i, remainder) = xg_i.divide_by_vanishing_poly(*src_domain).unwrap();
            assert!(remainder.is_zero());
            end_timer!(selector_time);
            Ok((h_i, Some(xg_i)))
        }
    }
}

fn inner_product<F: PrimeField>(
    public_variables: &[F],
    private_variables: &[F],
    row: &[(F, usize)],
    num_public_variables: usize,
) -> F {
    let mut result = F::zero();

    for &(ref coefficient, i) in row {
        // Fetch the variable.
        let variable = match i < num_public_variables {
            true => public_variables[i],
            false => private_variables[i - num_public_variables],
        };

        result += if coefficient.is_one() { variable } else { variable * coefficient };
    }

    result
}

#[test]
fn check_division_by_vanishing_poly_preserve_sparseness() {
    use crate::fft::{EvaluationDomain, Evaluations as EvaluationsOnDomain};
    use snarkvm_curves::bls12_377::Fr;
    use snarkvm_fields::{Field, One, Zero};

    let domain = EvaluationDomain::new(16).unwrap();
    let small_domain = EvaluationDomain::new(4).unwrap();
    let val = Fr::one().double().double().double() - Fr::one();
    let mut evals = (0..16).map(|pow| val.pow([pow])).collect::<Vec<_>>();
    for i in 0..4 {
        evals[4 * i] = Fr::zero();
    }
    let p = EvaluationsOnDomain::from_vec_and_domain(evals, domain).interpolate();
    assert_eq!(p.degree(), 15);
    let (p_div_v, p_mod_v) = p.divide_by_vanishing_poly(small_domain).unwrap();
    assert!(p_mod_v.is_zero());
    dbg!(p_div_v.degree());
    dbg!(p_div_v.evaluate_over_domain(domain));
}
