use crate::ctx::Canceller;
use crate::ctx::Context;
use crate::dbs::Options;
use crate::dbs::Statement;
use crate::doc::Document;
use crate::err::Error;
use crate::idx::planner::plan::Plan;
use crate::sql::array::Array;
use crate::sql::edges::Edges;
use crate::sql::field::Field;
use crate::sql::range::Range;
use crate::sql::table::Table;
use crate::sql::thing::Thing;
use crate::sql::value::Value;
use crate::sql::Object;
use async_recursion::async_recursion;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::mem;

pub(crate) enum Iterable {
	Value(Value),
	Table(Table),
	Thing(Thing),
	Range(Range),
	Edges(Edges),
	Mergeable(Thing, Value),
	Relatable(Thing, Thing, Thing),
	Index(Table, Plan),
}

pub(crate) enum Operable {
	Value(Value),
	Mergeable(Value, Value),
	Relatable(Thing, Value, Thing),
}

pub(crate) enum Workable {
	Normal,
	Insert(Value),
	Relate(Thing, Thing),
}

#[derive(Default)]
pub(crate) struct Iterator {
	// Iterator status
	run: Canceller,
	// Iterator limit value
	limit: Option<usize>,
	// Iterator start value
	start: Option<usize>,
	// Iterator runtime error
	error: Option<Error>,
	// Iterator output results
	results: Vec<Value>,
	// Iterator input values
	entries: Vec<Iterable>,
}

impl Iterator {
	/// Creates a new iterator
	pub fn new() -> Self {
		Self::default()
	}

	/// Prepares a value for processing
	pub fn ingest(&mut self, val: Iterable) {
		self.entries.push(val)
	}

	/// Process the records and output
	pub async fn output(
		&mut self,
		ctx: &Context<'_>,
		opt: &Options,
		stm: &Statement<'_>,
	) -> Result<Value, Error> {
		// Log the statement
		trace!("Iterating: {}", stm);
		// Enable context override
		let mut cancel_ctx = Context::new(ctx);
		self.run = cancel_ctx.add_cancel();
		// Process the query LIMIT clause
		self.setup_limit(&cancel_ctx, opt, stm).await?;
		// Process the query START clause
		self.setup_start(&cancel_ctx, opt, stm).await?;
		// Process any EXPLAIN clause
		let explanation = self.output_explain(&cancel_ctx, opt, stm)?;
		// Process prepared values
		self.iterate(&cancel_ctx, opt, stm).await?;
		// Return any document errors
		if let Some(e) = self.error.take() {
			return Err(e);
		}
		// Process any SPLIT clause
		self.output_split(ctx, opt, stm).await?;
		// Process any GROUP clause
		self.output_group(ctx, opt, stm).await?;
		// Process any ORDER clause
		self.output_order(ctx, opt, stm).await?;
		// Process any START clause
		self.output_start(ctx, opt, stm).await?;
		// Process any LIMIT clause
		self.output_limit(ctx, opt, stm).await?;
		// Process any FETCH clause
		self.output_fetch(ctx, opt, stm).await?;
		// Add the EXPLAIN clause to the result
		if let Some(e) = explanation {
			self.results.push(e);
		}
		// Output the results
		Ok(mem::take(&mut self.results).into())
	}

	#[inline]
	async fn setup_limit(
		&mut self,
		ctx: &Context<'_>,
		opt: &Options,
		stm: &Statement<'_>,
	) -> Result<(), Error> {
		if let Some(v) = stm.limit() {
			self.limit = Some(v.process(ctx, opt).await?);
		}
		Ok(())
	}

	#[inline]
	async fn setup_start(
		&mut self,
		ctx: &Context<'_>,
		opt: &Options,
		stm: &Statement<'_>,
	) -> Result<(), Error> {
		if let Some(v) = stm.start() {
			self.start = Some(v.process(ctx, opt).await?);
		}
		Ok(())
	}

	#[inline]
	async fn output_split(
		&mut self,
		ctx: &Context<'_>,
		opt: &Options,
		stm: &Statement<'_>,
	) -> Result<(), Error> {
		if let Some(splits) = stm.split() {
			// Loop over each split clause
			for split in splits.iter() {
				// Get the query result
				let res = mem::take(&mut self.results);
				// Loop over each value
				for obj in &res {
					// Get the value at the path
					let val = obj.pick(split);
					// Set the value at the path
					match val {
						Value::Array(v) => {
							for val in v {
								// Make a copy of object
								let mut obj = obj.clone();
								// Set the value at the path
								obj.set(ctx, opt, split, val).await?;
								// Add the object to the results
								self.results.push(obj);
							}
						}
						_ => {
							// Make a copy of object
							let mut obj = obj.clone();
							// Set the value at the path
							obj.set(ctx, opt, split, val).await?;
							// Add the object to the results
							self.results.push(obj);
						}
					}
				}
			}
		}
		Ok(())
	}

	#[inline]
	async fn output_group(
		&mut self,
		ctx: &Context<'_>,
		opt: &Options,
		stm: &Statement<'_>,
	) -> Result<(), Error> {
		if let Some(fields) = stm.expr() {
			if let Some(groups) = stm.group() {
				// Create the new grouped collection
				let mut grp: BTreeMap<Array, Array> = BTreeMap::new();
				// Get the query result
				let res = mem::take(&mut self.results);
				// Loop over each value
				for obj in res {
					// Create a new column set
					let mut arr = Array::with_capacity(groups.len());
					// Loop over each group clause
					for group in groups.iter() {
						// Get the value at the path
						let val = obj.pick(group);
						// Set the value at the path
						arr.push(val);
					}
					// Add to grouped collection
					match grp.get_mut(&arr) {
						Some(v) => v.push(obj),
						None => {
							grp.insert(arr, Array::from(obj));
						}
					}
				}
				// Loop over each grouped collection
				for (_, vals) in grp {
					// Create a new value
					let mut obj = Value::base();
					// Save the collected values
					let vals = Value::from(vals);
					// Loop over each group clause
					for field in fields.other() {
						// Process the field
						if let Field::Single {
							expr,
							alias,
						} = field
						{
							let idiom = alias
								.as_ref()
								.map(Cow::Borrowed)
								.unwrap_or_else(|| Cow::Owned(expr.to_idiom()));
							match expr {
								Value::Function(f) if f.is_aggregate() => {
									let x = vals.all().get(ctx, opt, idiom.as_ref()).await?;
									let x = f.aggregate(x).compute(ctx, opt).await?;
									obj.set(ctx, opt, idiom.as_ref(), x).await?;
								}
								_ => {
									let x = vals.first();
									let mut child_ctx = Context::new(ctx);
									child_ctx.add_cursor_doc(&x);
									let x = if let Some(alias) = alias {
										alias.compute(&child_ctx, opt).await?
									} else {
										expr.compute(&child_ctx, opt).await?
									};
									obj.set(ctx, opt, idiom.as_ref(), x).await?;
								}
							}
						}
					}
					// Add the object to the results
					self.results.push(obj);
				}
			}
		}
		Ok(())
	}

	#[inline]
	async fn output_order(
		&mut self,
		_ctx: &Context<'_>,
		_opt: &Options,
		stm: &Statement<'_>,
	) -> Result<(), Error> {
		if let Some(orders) = stm.order() {
			// Sort the full result set
			self.results.sort_by(|a, b| {
				// Loop over each order clause
				for order in orders.iter() {
					// Reverse the ordering if DESC
					let o = match order.random {
						true => {
							let a = rand::random::<f64>();
							let b = rand::random::<f64>();
							a.partial_cmp(&b)
						}
						false => match order.direction {
							true => a.compare(b, order, order.collate, order.numeric),
							false => b.compare(a, order, order.collate, order.numeric),
						},
					};
					//
					match o {
						Some(Ordering::Greater) => return Ordering::Greater,
						Some(Ordering::Equal) => continue,
						Some(Ordering::Less) => return Ordering::Less,
						None => continue,
					}
				}
				Ordering::Equal
			})
		}
		Ok(())
	}

	#[inline]
	async fn output_start(
		&mut self,
		_ctx: &Context<'_>,
		_opt: &Options,
		_stm: &Statement<'_>,
	) -> Result<(), Error> {
		if let Some(v) = self.start {
			self.results = mem::take(&mut self.results).into_iter().skip(v).collect();
		}
		Ok(())
	}

	#[inline]
	async fn output_limit(
		&mut self,
		_ctx: &Context<'_>,
		_opt: &Options,
		_stm: &Statement<'_>,
	) -> Result<(), Error> {
		if let Some(v) = self.limit {
			self.results = mem::take(&mut self.results).into_iter().take(v).collect();
		}
		Ok(())
	}

	#[inline]
	async fn output_fetch(
		&mut self,
		ctx: &Context<'_>,
		opt: &Options,
		stm: &Statement<'_>,
	) -> Result<(), Error> {
		if let Some(fetchs) = stm.fetch() {
			for fetch in fetchs.iter() {
				// Loop over each result value
				for obj in &mut self.results {
					// Fetch the value at the path
					obj.fetch(ctx, opt, fetch).await?;
				}
			}
		}
		Ok(())
	}

	#[inline]
	fn output_explain(
		&mut self,
		_ctx: &Context<'_>,
		_opt: &Options,
		stm: &Statement<'_>,
	) -> Result<Option<Value>, Error> {
		Ok(if stm.explain() {
			let mut explains = Vec::with_capacity(self.entries.len());
			for iter in &self.entries {
				let (operation, detail) = match iter {
					Iterable::Value(v) => ("Iterate Value", vec![("value", v.to_owned())]),
					Iterable::Table(t) => {
						("Iterate Table", vec![("table", Value::from(t.0.to_owned()))])
					}
					Iterable::Thing(t) => {
						("Iterate Thing", vec![("thing", Value::Thing(t.to_owned()))])
					}
					Iterable::Range(r) => {
						("Iterate Range", vec![("table", Value::from(r.tb.to_owned()))])
					}
					Iterable::Edges(e) => {
						("Iterate Edges", vec![("from", Value::Thing(e.from.to_owned()))])
					}
					Iterable::Mergeable(t, v) => (
						"Iterate Mergeable",
						vec![("thing", Value::Thing(t.to_owned())), ("value", v.to_owned())],
					),
					Iterable::Relatable(t1, t2, t3) => (
						"Iterate Relatable",
						vec![
							("thing-1", Value::Thing(t1.to_owned())),
							("thing-2", Value::Thing(t2.to_owned())),
							("thing-3", Value::Thing(t3.to_owned())),
						],
					),
					Iterable::Index(t, p) => (
						"Iterate Index",
						vec![("table", Value::from(t.0.to_owned())), ("plan", p.explain())],
					),
				};
				let explain = Object::from(HashMap::from([
					("operation", Value::from(operation)),
					("detail", Value::Object(Object::from(HashMap::from_iter(detail)))),
				]));
				explains.push(Value::Object(explain));
			}
			Some(Value::Object(Object::from(HashMap::from([(
				"explain",
				Value::Array(Array::from(explains)),
			)]))))
		} else {
			None
		})
	}

	#[cfg(target_arch = "wasm32")]
	#[async_recursion(?Send)]
	async fn iterate(
		&mut self,
		ctx: &Context<'_>,
		opt: &Options,
		stm: &Statement<'_>,
	) -> Result<(), Error> {
		// Prevent deep recursion
		let opt = &opt.dive(4)?;
		// Process all prepared values
		for v in mem::take(&mut self.entries) {
			v.iterate(ctx, opt, stm, self).await?;
		}
		// Everything processed ok
		Ok(())
	}

	#[cfg(not(target_arch = "wasm32"))]
	#[async_recursion]
	async fn iterate(
		&mut self,
		ctx: &Context<'_>,
		opt: &Options,
		stm: &Statement<'_>,
	) -> Result<(), Error> {
		// Prevent deep recursion
		let opt = &opt.dive(4)?;
		// Check if iterating in parallel
		match stm.parallel() {
			// Run statements sequentially
			false => {
				// Process all prepared values
				for v in mem::take(&mut self.entries) {
					v.iterate(ctx, opt, stm, self).await?;
				}
				// Everything processed ok
				Ok(())
			}
			// Run statements in parallel
			true => {
				// Create a new executor
				let e = executor::Executor::new();
				// Take all of the iterator values
				let vals = mem::take(&mut self.entries);
				// Create a channel to shutdown
				let (end, exit) = channel::bounded::<()>(1);
				// Create an unbounded channel
				let (chn, docs) = channel::bounded(crate::cnf::MAX_CONCURRENT_TASKS);
				// Create an async closure for prepared values
				let adocs = async {
					// Process all prepared values
					for v in vals {
						e.spawn(v.channel(ctx, opt, stm, chn.clone()))
							// Ensure we detach the spawned task
							.detach();
					}
					// Drop the uncloned channel instance
					drop(chn);
				};
				// Create an unbounded channel
				let (chn, vals) = channel::bounded(crate::cnf::MAX_CONCURRENT_TASKS);
				// Create an async closure for received values
				let avals = async {
					// Process all received values
					while let Ok((k, v)) = docs.recv().await {
						e.spawn(Document::compute(ctx, opt, stm, chn.clone(), k, v))
							// Ensure we detach the spawned task
							.detach();
					}
					// Drop the uncloned channel instance
					drop(chn);
				};
				// Create an async closure to process results
				let aproc = async {
					// Process all processed values
					while let Ok(r) = vals.recv().await {
						self.result(r, stm);
					}
					// Shutdown the executor
					let _ = end.send(()).await;
				};
				// Run all executor tasks
				let fut = e.run(exit.recv());
				// Wait for all closures
				let res = futures::join!(adocs, avals, aproc, fut);
				// Consume executor error
				let _ = res.3;
				// Everything processed ok
				Ok(())
			}
		}
	}

	/// Process a new record Thing and Value
	pub async fn process(
		&mut self,
		ctx: &Context<'_>,
		opt: &Options,
		stm: &Statement<'_>,
		val: Operable,
	) {
		// Check current context
		if ctx.is_done() {
			return;
		}
		// Setup a new workable
		let (val, ext) = match val {
			Operable::Value(v) => (v, Workable::Normal),
			Operable::Mergeable(v, o) => (v, Workable::Insert(o)),
			Operable::Relatable(f, v, w) => (v, Workable::Relate(f, w)),
		};
		// Setup a new document
		let mut doc = Document::new(ctx.thing(), &val, ext);
		// Process the document
		let res = match stm {
			Statement::Select(_) => doc.select(ctx, opt, stm).await,
			Statement::Create(_) => doc.create(ctx, opt, stm).await,
			Statement::Update(_) => doc.update(ctx, opt, stm).await,
			Statement::Relate(_) => doc.relate(ctx, opt, stm).await,
			Statement::Delete(_) => doc.delete(ctx, opt, stm).await,
			Statement::Insert(_) => doc.insert(ctx, opt, stm).await,
			_ => unreachable!(),
		};
		// Process the result
		self.result(res, stm);
	}

	/// Accept a processed record result
	fn result(&mut self, res: Result<Value, Error>, stm: &Statement<'_>) {
		// Process the result
		match res {
			Err(Error::Ignore) => {
				return;
			}
			Err(e) => {
				self.error = Some(e);
				self.run.cancel();
				return;
			}
			Ok(v) => self.results.push(v),
		}
		// Check if we can exit
		if stm.group().is_none() && stm.order().is_none() {
			if let Some(l) = self.limit {
				if let Some(s) = self.start {
					if self.results.len() == l + s {
						self.run.cancel()
					}
				} else if self.results.len() == l {
					self.run.cancel()
				}
			}
		}
	}
}
