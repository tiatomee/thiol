// SPDX-FileCopyrightText: 2021 The thiol developers
//
// SPDX-License-Identifier: EUPL-1.2

use std::collections::{BTreeMap, HashMap, HashSet};

use hir::{Expression, FileLocation, Function, Identifier, TypeDefinition, VariableDef};
use thiol_hir::{self as hir, TypeReference};

use bimap::BiBTreeMap;
use id_arena::Id;

pub mod types;
pub use types::*;

pub enum Error {
    TypeRedefinition {
        previous_name: FileLocation,
        redefinition_name: FileLocation,
        redefinition: FileLocation,
    },

    GenericParamaterRedefinition {
        previous_name: FileLocation,
        redefinition: FileLocation,
    },

    RecursiveTypeDefinition {
        type_def: FileLocation,
        type_name: FileLocation,
        recurive_usages: Vec<FileLocation>,
    },
    MutuallyRecursiveTypeDefinitions {
        type_def_idents: Vec<FileLocation>,
    },

    UndefinedType {
        name: String,
        uses: Vec<FileLocation>,
    },

    HigherKindedGenericTypeUsed {
        loc: FileLocation,
        generic_name: Identifier,
    },
    MismatchedNumberGenericArgs {
        loc: FileLocation,
        given: usize,
        expected: usize,
        def_loc: FileLocation,
    },

    FieldRedefinition {
        previous_name: FileLocation,
        redefinition_name: FileLocation,
        item: FileLocation,
    },

    FunctionRedefinition {
        previous_name: FileLocation,
        previous_sig: FileLocation,
        redefinition_name: FileLocation,
        redefinition_sig: FileLocation,
    },
    ConstantRedefinition {
        previous_name: FileLocation,
        previous_def: FileLocation,
        redefinition_name: FileLocation,
        redefinition_def: FileLocation,
    },
}

pub fn type_check(
    ty_ctx: &mut Context,
    hir_ctx: &hir::Context,
    module: &hir::Module,
) -> Result<(), Vec<Error>> {
    process_type_definitions(module, ty_ctx, hir_ctx)?;

    add_function_signatures(module, ty_ctx, hir_ctx)?;

    add_constants(module, ty_ctx, hir_ctx)?;

    Ok(())
}

fn add_constants(
    module: &hir::Module,
    ty_ctx: &mut Context,
    hir_ctx: &hir::Context,
) -> Result<(), Vec<Error>> {
    let mut errs = vec![];

    for c in &module.consts {
        if let Err(err) = ty_ctx.add_constant(hir_ctx, *c) {
            errs.push(err);
        }
    }

    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

fn add_function_signatures(
    module: &hir::Module,
    ty_ctx: &mut Context,
    hir_ctx: &hir::Context,
) -> Result<(), Vec<Error>> {
    let mut errors = vec![];
    for func in &module.functions {
        if let Err(err) = ty_ctx.add_function_signature(hir_ctx, *func) {
            errors.push(err);
        }
    }

    if !errors.is_empty() {
        Err(errors)
    } else {
        Ok(())
    }
}

fn process_type_definitions(
    module: &hir::Module,
    ty_ctx: &mut Context,
    hir_ctx: &hir::Context,
) -> Result<(), Vec<Error>> {
    let mut errs = vec![];

    // sort type definitions by dependency
    let mut tyname_to_node = HashMap::new();
    let mut g =
        petgraph::graph::Graph::<Option<Id<TypeDefinition>>, petgraph::graph::NodeIndex>::new();

    let mut deps = HashMap::new();

    for ty in &module.types {
        let ty_def = &hir_ctx.type_defs[*ty];
        let ty_name = &hir_ctx.identifiers[ty_def.name];

        let node = g.add_node(Some(*ty));

        // definition with the same name
        if let Some(prev_idx) = tyname_to_node.insert(ty_name.clone(), node) {
            let prev_id = g[prev_idx].unwrap();
            let prev_def = &hir_ctx.type_defs[prev_id];

            errs.push(Error::TypeRedefinition {
                previous_name: hir_ctx.identifier_fcs[&prev_def.name],
                redefinition_name: hir_ctx.identifier_fcs[&ty_def.name],
                redefinition: hir_ctx.type_def_fcs[ty],
            });
            continue;
        }
    }

    for ty in &module.types {
        let def = &hir_ctx.type_defs[*ty];
        let ty_name = &hir_ctx.identifiers[def.name];
        if let Err(err) = type_def_deps(hir_ctx, def, &mut deps) {
            errs.push(err);
            continue;
        }

        if let Some(usages) = deps.get(ty_name.as_str()) {
            errs.push(Error::RecursiveTypeDefinition {
                type_def: hir_ctx.type_def_fcs[ty],
                type_name: hir_ctx.identifier_fcs[&def.name],
                recurive_usages: usages.clone(),
            });
            continue;
        }

        let self_node = tyname_to_node[ty_name];

        for (name, uses) in deps.drain() {
            if let Some(id) = tyname_to_node.get(name) {
                g.add_edge(self_node, *id, Default::default());
            } else {
                errs.push(Error::UndefinedType {
                    name: name.to_string(),
                    uses,
                });
                continue;
            };
        }
    }

    if !errs.is_empty() {
        return Err(errs);
    }

    let groups = petgraph::algo::tarjan_scc(&g);

    for group in groups {
        if group.len() > 1 {
            errs.push(Error::MutuallyRecursiveTypeDefinitions {
                type_def_idents: group
                    .into_iter()
                    .map(|id| {
                        let id = g[id].unwrap();
                        hir_ctx.identifier_fcs[&hir_ctx.type_defs[id].name]
                    })
                    .collect(),
            });
            continue;
        }

        debug_assert_eq!(group.len(), 1);

        let id = g[group[0]].unwrap();

        if let Err(errors) = ty_ctx.add_type_definition(hir_ctx, id) {
            errs.extend(errors);
        }
    }

    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

#[derive(Default, Clone)]
pub struct Context {
    pub defs: BTreeMap<Identifier, Id<TypeDefinition>>,
    pub generic_distinct_ids: BTreeMap<Identifier, usize>,

    // generic types are *incomplete* before they are applied, but
    // non generic types will be able to be mapped directly to a type
    pub complete_types: BTreeMap<Identifier, TypeId>,

    pub types: BiBTreeMap<Type, TypeId>,
    pub distinct_counter: usize,

    pub function_sigs: BTreeMap<Identifier, FunctionSig>,
    pub consts: BTreeMap<Identifier, ConstantSig>,
}

impl Context {
    fn ty_ref(
        &mut self,
        ctx: &hir::Context,
        id: Id<TypeReference>,
        subst: &HashMap<&str, TypeId>,
    ) -> Result<TypeId, Error> {
        use hir::PrimitiveType as PT;
        use TypeReference as TR;

        let ty_ref = &ctx.type_refs[id];

        let ty = match ty_ref {
            TR::Primitive(prim) => match prim {
                PT::Bool => Type::Bool,
                PT::Int => Type::Int,
                PT::UInt => Type::UInt,
                PT::Float => Type::Float,
                PT::Double => Type::Double,
                PT::BoolVec { components } => Type::BoolVec {
                    components: (*components).into(),
                },
                PT::IntVec {
                    components,
                    vtype,
                    space,
                } => Type::IntVec {
                    components: (*components).into(),
                    vtype: vtype.map(|ty| ctx.vec_types[ty]).into(),
                    space: space.map(|id| ctx.identifiers[id].clone()),
                },
                PT::UIntVec {
                    components,
                    vtype,
                    space,
                } => Type::UIntVec {
                    components: (*components).into(),
                    vtype: vtype.map(|ty| ctx.vec_types[ty]).into(),
                    space: space.map(|id| ctx.identifiers[id].clone()),
                },
                PT::FloatVec {
                    components,
                    vtype,
                    space,
                } => Type::FloatVec {
                    components: (*components).into(),
                    vtype: vtype.map(|ty| ctx.vec_types[ty]).into(),
                    space: space.map(|id| ctx.identifiers[id].clone()),
                },
                PT::DoubleVec {
                    components,
                    vtype,
                    space,
                } => Type::DoubleVec {
                    components: (*components).into(),
                    vtype: vtype.map(|ty| ctx.vec_types[ty]).into(),
                    space: space.map(|id| ctx.identifiers[id].clone()),
                },
                PT::FloatMat {
                    cols,
                    rows,
                    transform,
                } => Type::FloatMat {
                    cols: (*cols).into(),
                    rows: (*rows).into(),
                    transform: transform.map(|(from, to)| {
                        (ctx.identifiers[from].clone(), ctx.identifiers[to].clone())
                    }),
                },
                PT::DoubleMat {
                    cols,
                    rows,
                    transform,
                } => Type::DoubleMat {
                    cols: (*cols).into(),
                    rows: (*rows).into(),
                    transform: transform.map(|(from, to)| {
                        (ctx.identifiers[from].clone(), ctx.identifiers[to].clone())
                    }),
                },
            },
            TR::OpenArray(inner) => {
                let inner_id = self.ty_ref(ctx, *inner, subst)?;
                Type::OpenArray { base: inner_id }
            }
            TR::Array { base, size } => {
                let inner_id = self.ty_ref(ctx, *base, subst)?;
                Type::Array {
                    base: inner_id,
                    size: *size,
                }
            }
            TR::Named { name, generics } => {
                let loc = ctx.type_ref_fcs[&id];
                let mut gens = Vec::with_capacity(generics.len());
                for id in generics {
                    let id: TypeId = self.ty_ref(ctx, *id, subst)?;
                    gens.push(id);
                }

                let name = &ctx.identifiers[*name];

                if let Some(subst_id) = subst.get(name.as_str()) {
                    if !gens.is_empty() {
                        return Err(Error::HigherKindedGenericTypeUsed {
                            generic_name: name.clone(),
                            loc,
                        });
                    }
                    return Ok(*subst_id);
                } else {
                    return self.ty_named(ctx, loc, name, &gens);
                }
            }
        };

        Ok(self.add_or_get_type(ty))
    }

    fn add_type_definition(
        &mut self,
        ctx: &hir::Context,
        id: Id<TypeDefinition>,
    ) -> Result<(), Vec<Error>> {
        let def = &ctx.type_defs[id];
        let def_loc = ctx.type_def_fcs[&id];
        let name = &ctx.identifiers[def.name];

        let old = self.defs.insert(name.clone(), id);
        debug_assert!(old.is_none());

        // "complete" types (types without generics) can be stored separately and
        // already be translated (instead of only validated)
        if def.generics.is_empty() {
            let ty_id = match &ctx.type_def_rhss[def.rhs] {
                hir::TypeDefinitionRhs::Distinct(id) => {
                    let alias_id = self
                        .ty_ref(ctx, *id, &Default::default())
                        .map_err(|err| vec![err])?;

                    let distinct_id = self.next_distinct_id();
                    self.add_type(Type::Distinct {
                        distinct_id,
                        inner: alias_id,
                    })
                }
                hir::TypeDefinitionRhs::Alias(id) => self
                    .ty_ref(ctx, *id, &Default::default())
                    .map_err(|err| vec![err])?,
                hir::TypeDefinitionRhs::Record { fields: field_ids } => {
                    let mut errs = vec![];
                    let mut fields_so_far = HashMap::new();

                    let mut fields = vec![];

                    for field in field_ids {
                        let var_def = &ctx.variable_defs[*field];
                        let var_name = &ctx.identifiers[var_def.name];
                        let var_fc = ctx.identifier_fcs[&var_def.name];

                        if let Some(prev) = fields_so_far.insert(var_name.as_str(), var_fc) {
                            errs.push(Error::FieldRedefinition {
                                item: def_loc,
                                previous_name: prev,
                                redefinition_name: var_fc,
                            });
                        }

                        match self.ty_ref(ctx, var_def.type_, &Default::default()) {
                            Ok(id) => {
                                fields.push((var_name.clone(), id));
                            }
                            Err(err) => {
                                errs.push(err);
                            }
                        }
                    }

                    if !errs.is_empty() {
                        return Err(errs);
                    }

                    let inner = self.add_or_get_type(Type::Record { fields });
                    let distinct_id = self.next_distinct_id();
                    self.add_type(Type::Distinct { distinct_id, inner })
                }
            };
            let old = self.complete_types.insert(name.clone(), ty_id);
            debug_assert!(old.is_none());
            Ok(())
        } else {
            let generics = def
                .generics
                .iter()
                .map(|id| ctx.identifiers[*id].as_str())
                .collect();

            let mut errs = vec![];
            match &ctx.type_def_rhss[def.rhs] {
                hir::TypeDefinitionRhs::Distinct(id) => {
                    if let Err(err) = self.ty_validate_ref(ctx, *id, &generics) {
                        errs.push(err);
                    }

                    let distinct_id = self.next_distinct_id();
                    self.generic_distinct_ids.insert(name.clone(), distinct_id);
                }
                hir::TypeDefinitionRhs::Alias(id) => {
                    if let Err(err) = self.ty_validate_ref(ctx, *id, &generics) {
                        errs.push(err);
                    }
                }
                hir::TypeDefinitionRhs::Record { fields } => {
                    let mut fields_so_far = HashMap::new();

                    for field in fields {
                        let var_def = &ctx.variable_defs[*field];
                        let var_name = &ctx.identifiers[var_def.name];
                        let var_fc = ctx.identifier_fcs[&var_def.name];

                        if let Some(prev) = fields_so_far.insert(var_name.as_str(), var_fc) {
                            errs.push(Error::FieldRedefinition {
                                item: def_loc,
                                previous_name: prev,
                                redefinition_name: var_fc,
                            });
                        }

                        if let Err(err) = self.ty_validate_ref(ctx, var_def.type_, &generics) {
                            errs.push(err);
                        }
                    }

                    let distinct_id = self.next_distinct_id();
                    self.generic_distinct_ids.insert(name.clone(), distinct_id);
                }
            }
            if errs.is_empty() {
                Ok(())
            } else {
                Err(errs)
            }
        }
    }

    fn add_function_signature(
        &mut self,
        ctx: &hir::Context,
        func: Id<Function>,
    ) -> Result<(), Error> {
        let fun = &ctx.functions[func];

        let ret = self.ty_ref(ctx, fun.ret_type, &Default::default())?;

        let args = fun
            .args
            .iter()
            .map(|(nam, ty)| {
                let ident = ctx.identifiers[*nam].clone();
                let ty = self.ty_ref(ctx, *ty, &Default::default())?;
                Ok((ident, ty))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let sig = FunctionSig {
            func_id: func,
            args,
            ret,
        };
        let name = ctx.identifiers[fun.name].clone();

        use std::collections::btree_map::Entry;
        match self.function_sigs.entry(name) {
            Entry::Vacant(entry) => {
                entry.insert(sig);
                Ok(())
            }
            Entry::Occupied(entry) => {
                let prev_id = entry.get().func_id;
                let prev_func = &ctx.functions[prev_id];
                let prev_item_fc = ctx.function_fcs[&prev_id];
                let prev_name_fc = ctx.identifier_fcs[&prev_func.name];

                let redef_item_fc = ctx.function_fcs[&func];
                let redef_name_fc = ctx.identifier_fcs[&fun.name];

                Err(Error::FunctionRedefinition {
                    previous_name: prev_name_fc,
                    previous_sig: prev_item_fc,
                    redefinition_name: redef_name_fc,
                    redefinition_sig: redef_item_fc,
                })
            }
        }
    }

    fn add_constant(&mut self, hir_ctx: &hir::Context, id: Id<VariableDef>) -> Result<(), Error> {
        let def = &hir_ctx.variable_defs[id];
        let name = &hir_ctx.identifiers[def.name];

        let ty = self.ty_ref(hir_ctx, def.type_, &Default::default())?;

        match self.consts.entry(name.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(ConstantSig {
                    const_id: id,
                    type_: ty,
                });
                Ok(())
            }
            std::collections::btree_map::Entry::Occupied(entry) => {
                let prev_sig = entry.get();
                let prev = &hir_ctx.variable_defs[prev_sig.const_id];

                let redef_name = hir_ctx.identifier_fcs[&def.name];
                let redef_def = hir_ctx.variable_def_fcs[&id];

                let prev_name = hir_ctx.identifier_fcs[&prev.name];
                let prev_def = hir_ctx.variable_def_fcs[&prev_sig.const_id];

                Err(Error::ConstantRedefinition {
                    previous_def: prev_def,
                    previous_name: prev_name,

                    redefinition_def: redef_def,
                    redefinition_name: redef_name,
                })
            }
        }
    }

    #[allow(dead_code, unused_variables)]
    fn check_expression(
        &mut self,
        ctx: &hir::Context,
        expr: Id<Expression>,
        ty: &Type,
    ) -> Result<(), Error> {
        match &ctx.expressions[expr] {
            Expression::Literal(_) => {}
            Expression::Variable(_) => {}
            Expression::PrimitiveOp(_) => {}
            Expression::Call {
                name,
                pos_args,
                nam_args,
            } => {}
            Expression::Field { base, name } => {}
            Expression::Index { base, index } => {}
            Expression::As { base, ty } => {}
        }
        todo!()
    }

    fn add_type(&mut self, ty: Type) -> TypeId {
        let next_id = TypeId(self.types.len());

        let res = self.types.insert(ty, next_id);
        debug_assert_eq!(res.did_overwrite(), false);
        next_id
    }

    fn add_or_get_type(&mut self, ty: Type) -> TypeId {
        if let Some(id) = self.types.get_by_left(&ty) {
            *id
        } else {
            self.add_type(ty)
        }
    }

    fn ty_named(
        &mut self,
        ctx: &hir::Context,
        loc: FileLocation,
        name: &str,
        generics: &[TypeId],
    ) -> Result<TypeId, Error> {
        // fast path: check if the type is a known complete type
        if let Some(ty) = self.complete_types.get(name) {
            if generics.is_empty() {
                return Ok(*ty);
            } else {
                // There was a complete type but this one uses generics!
                // Instead of duplicating the error handling logic we continue
                // on the non-complete path
            }
        }

        if let Some(id) = self.defs.get(name) {
            let def_loc = ctx.type_def_fcs[id];
            let def = &ctx.type_defs[*id];

            if def.generics.len() != generics.len() {
                Err(Error::MismatchedNumberGenericArgs {
                    loc,
                    expected: def.generics.len(),
                    given: generics.len(),
                    def_loc,
                })
            } else {
                let subst = def
                    .generics
                    .iter()
                    .map(|id| ctx.identifiers[*id].as_str())
                    .zip(generics.iter().copied())
                    .collect();

                match &ctx.type_def_rhss[def.rhs] {
                    hir::TypeDefinitionRhs::Distinct(id) => {
                        let inner = self.ty_ref(ctx, *id, &subst)?;
                        let distinct_id = self.generic_distinct_ids[name];
                        Ok(self.add_or_get_type(Type::Distinct { distinct_id, inner }))
                    }
                    hir::TypeDefinitionRhs::Alias(id) => self.ty_ref(ctx, *id, &subst),
                    hir::TypeDefinitionRhs::Record { fields } => {
                        let distinct_id = self.generic_distinct_ids[name];
                        let mut record_fields = Vec::with_capacity(fields.len());
                        for field in fields {
                            let def = &ctx.variable_defs[*field];
                            let name = ctx.identifiers[def.name].clone();
                            let field_ty = self.ty_ref(ctx, def.type_, &subst)?;
                            record_fields.push((name, field_ty));
                        }

                        let inner = self.add_or_get_type(Type::Record {
                            fields: record_fields,
                        });

                        Ok(self.add_or_get_type(Type::Distinct { distinct_id, inner }))
                    }
                }
            }
        } else {
            Err(Error::UndefinedType {
                name: name.to_string(),
                uses: vec![loc],
            })
        }
    }

    /// Validate a type reference
    ///
    /// Used to check that a type definition is valid without having to instantiate
    /// generics
    fn ty_validate_ref(
        &self,
        ctx: &hir::Context,
        id: Id<TypeReference>,
        generics: &HashSet<&str>,
    ) -> Result<(), Error> {
        let ty_ref = &ctx.type_refs[id];

        match ty_ref {
            TypeReference::Primitive(_) => Ok(()),
            TypeReference::OpenArray(inner) => self.ty_validate_ref(ctx, *inner, generics),
            TypeReference::Array { base, size: _ } => self.ty_validate_ref(ctx, *base, generics),
            TypeReference::Named {
                name,
                generics: applied_gens,
            } => {
                let loc = ctx.type_ref_fcs[&id];
                let name_s = &ctx.identifiers[*name];
                if generics.contains(name_s.as_str()) {
                    if !applied_gens.is_empty() {
                        Err(Error::HigherKindedGenericTypeUsed {
                            generic_name: name_s.clone(),
                            loc,
                        })
                    } else {
                        Ok(())
                    }
                } else if let Some(def_id) = self.defs.get(name_s.as_str()) {
                    let def = &ctx.type_defs[*def_id];
                    let def_loc = ctx.type_def_fcs[def_id];
                    if def.generics.len() != applied_gens.len() {
                        Err(Error::MismatchedNumberGenericArgs {
                            loc,
                            expected: def.generics.len(),
                            given: applied_gens.len(),
                            def_loc,
                        })
                    } else {
                        for gen in applied_gens {
                            self.ty_validate_ref(ctx, *gen, generics)?;
                        }
                        Ok(())
                    }
                } else {
                    Err(Error::UndefinedType {
                        name: name_s.to_string(),
                        uses: vec![loc],
                    })
                }
            }
        }
    }

    fn next_distinct_id(&mut self) -> usize {
        let id = self.distinct_counter;
        self.distinct_counter += 1;
        id
    }
}

fn type_def_deps<'a>(
    ctx: &'a hir::Context,
    ty: &hir::TypeDefinition,
    deps: &mut HashMap<&'a str, Vec<FileLocation>>,
) -> Result<(), Error> {
    let mut generics = HashMap::new();

    for gen in &ty.generics {
        let name = &ctx.identifiers[*gen];
        let loc = ctx.identifier_fcs[gen];

        if let Some(prev) = generics.insert(name.as_str(), loc) {
            return Err(Error::GenericParamaterRedefinition {
                previous_name: prev,
                redefinition: loc,
            });
        }
    }

    let rhs = &ctx.type_def_rhss[ty.rhs];
    match rhs {
        hir::TypeDefinitionRhs::Distinct(ty) => {
            type_ref_deps(ctx, *ty, deps);
        }
        hir::TypeDefinitionRhs::Alias(ty) => {
            type_ref_deps(ctx, *ty, deps);
        }
        hir::TypeDefinitionRhs::Record { fields } => {
            for field in fields {
                let def = &ctx.variable_defs[*field];
                type_ref_deps(ctx, def.type_, deps);
            }
        }
    }

    for (gen, _) in generics {
        deps.remove(gen);
    }

    Ok(())
}

fn type_ref_deps<'a>(
    ctx: &'a hir::Context,
    ty: Id<hir::TypeReference>,
    deps: &mut HashMap<&'a str, Vec<FileLocation>>,
) {
    let ty_ref = &ctx.type_refs[ty];
    match ty_ref {
        TypeReference::Primitive(_) => {}
        TypeReference::OpenArray(base) => type_ref_deps(ctx, *base, deps),
        TypeReference::Array { base, size: _ } => type_ref_deps(ctx, *base, deps),
        TypeReference::Named { name, generics } => {
            let usage_loc = ctx.identifier_fcs[name];
            let name = &ctx.identifiers[*name];

            let locs = deps.entry(name.as_str()).or_default();
            locs.push(usage_loc);

            for gen in generics {
                type_ref_deps(ctx, *gen, deps);
            }
        }
    }
}
