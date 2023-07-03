use surrealdb::dbs::Session;
use surrealdb::err::Error;
use surrealdb::kvs::Datastore;
use surrealdb::sql::Value;
use tokio::time::Instant;

#[tokio::test]
async fn context_iteration_performance() -> Result<(), Error> {
	let dbs = Datastore::new("memory").await?;
	let ses = Session::for_kv().with_ns("test").with_db("test");
	let sql = format!(
		r"DEFINE INDEX number ON item FIELDS number;
		DEFINE ANALYZER simple TOKENIZERS blank,class;
		DEFINE INDEX search ON item FIELDS label SEARCH ANALYZER simple BM25"
	);
	let res = &mut dbs.execute(&sql, &ses, None, false).await?;
	for _ in 0..3 {
		assert!(res.remove(0).result.is_ok());
	}

	let count = 1000;

	{
		let mut total_time = 0;
		for i in 0..count {
			let j = i * 5;
			let a = j;
			let b = j + 1;
			let c = j + 2;
			let d = j + 3;
			let e = j + 4;
			let sql = format!(
				r"CREATE item SET id = {a}, name = '{a}', number = 0, label='alpha';
		CREATE item SET id = {b}, name = '{b}', number = 1, label='bravo';
		CREATE item SET id = {c}, name = '{c}', number = 2, label='charlie';
		CREATE item SET id = {d}, name = '{d}', number = 3, label='delta';
		CREATE item SET id = {e}, name = '{e}', number = 4, label='echo';",
			);
			let time = Instant::now();
			let res = &mut dbs.execute(&sql, &ses, None, false).await?;
			total_time += time.elapsed().as_micros();
			for _ in 0..5 {
				assert!(res.remove(0).result.is_ok());
			}
		}
		println!("INGESTING: {:?} micros", total_time / 5);
	}

	{
		let mut total_time = 0;
		for _ in 0..5 {
			let sql = format!("SELECT * FROM item");
			let time = Instant::now();
			let res = &mut dbs.execute(&sql, &ses, None, false).await?;
			total_time += time.elapsed().as_micros();
			let value = res.remove(0).result?;
			if let Value::Array(a) = value {
				assert_eq!(a.len(), count * 5);
			} else {
				panic!("Fail");
			}
		}
		println!("TABLE ITERATOR: {:?} micros", total_time / 5);
	}

	{
		let mut total_time = 0;
		for _ in 0..5 {
			let sql = format!("SELECT * FROM item WHERE number=4");
			let time = Instant::now();
			let res = &mut dbs.execute(&sql, &ses, None, false).await?;
			total_time += time.elapsed().as_micros();
			let value = res.remove(0).result?;
			if let Value::Array(a) = value {
				assert_eq!(a.len(), count);
			} else {
				panic!("Fail");
			}
		}
		println!("UNIQ INDEX_ITERATOR: {:?} micros", total_time / 5);
	}

	{
		let mut total_time = 0;
		for _ in 0..5 {
			let time = Instant::now();
			let sql = format!("SELECT * FROM item WHERE label @@ 'charlie'");
			let res = &mut dbs.execute(&sql, &ses, None, false).await?;
			total_time += time.elapsed().as_micros();
			let value = res.remove(0).result?;
			if let Value::Array(a) = value {
				assert_eq!(a.len(), count);
			} else {
				panic!("Fail");
			}
		}
		println!("SEARCH INDEX_ITERATOR: {:?} micros", total_time / 5);
	}
	Ok(())
}
