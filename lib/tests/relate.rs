mod parse;
use parse::Parse;
use surrealdb::dbs::Session;
use surrealdb::err::Error;
use surrealdb::kvs::Datastore;
use surrealdb::sql::Value;

#[tokio::test]
async fn relate_with_parameters() -> Result<(), Error> {
	let sql = "
		LET $tobie = person:tobie;
		LET $jaime = person:jaime;
		RELATE $tobie->knows->$jaime SET id = knows:test;
	";
	let dbs = Datastore::new("memory").await?;
	let ses = Session::for_kv().with_ns("test").with_db("test");
	let res = &mut dbs.execute(sql, &ses, None).await?;
	assert_eq!(res.len(), 3);
	//
	let tmp = res.remove(0).result?;
	let val = Value::None;
	assert_eq!(tmp, val);
	//
	let tmp = res.remove(0).result?;
	let val = Value::None;
	assert_eq!(tmp, val);
	//
	let tmp = res.remove(0).result?;
	let val = Value::parse(
		"[
			{
				id: knows:test,
				in: person:tobie,
				out: person:jaime,
			}
		]",
	);
	assert_eq!(tmp, val);
	//
	Ok(())
}

#[tokio::test]
async fn relate_and_overwrite() -> Result<(), Error> {
	let sql = "
		LET $tobie = person:tobie;
		LET $jaime = person:jaime;
		RELATE $tobie->knows->$jaime SET id = knows:test;
		UPDATE knows:test CONTENT { test: true };
		SELECT * FROM knows:test;
	";
	let dbs = Datastore::new("memory").await?;
	let ses = Session::for_kv().with_ns("test").with_db("test");
	let res = &mut dbs.execute(sql, &ses, None).await?;
	assert_eq!(res.len(), 5);
	//
	let tmp = res.remove(0).result?;
	let val = Value::None;
	assert_eq!(tmp, val);
	//
	let tmp = res.remove(0).result?;
	let val = Value::None;
	assert_eq!(tmp, val);
	//
	let tmp = res.remove(0).result?;
	let val = Value::parse(
		"[
			{
				id: knows:test,
				in: person:tobie,
				out: person:jaime,
			}
		]",
	);
	assert_eq!(tmp, val);
	//
	let tmp = res.remove(0).result?;
	let val = Value::parse(
		"[
			{
				id: knows:test,
				in: person:tobie,
				out: person:jaime,
				test: true,
			}
		]",
	);
	assert_eq!(tmp, val);
	//
	let tmp = res.remove(0).result?;
	let val = Value::parse(
		"[
			{
				id: knows:test,
				in: person:tobie,
				out: person:jaime,
				test: true,
			}
		]",
	);
	assert_eq!(tmp, val);
	//
	Ok(())
}
