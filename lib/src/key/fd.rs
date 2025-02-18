use derive::Key;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Serialize, Deserialize, Key)]
pub struct Fd<'a> {
	__: u8,
	_a: u8,
	pub ns: &'a str,
	_b: u8,
	pub db: &'a str,
	_c: u8,
	pub tb: &'a str,
	_d: u8,
	_e: u8,
	_f: u8,
	pub fd: &'a str,
}

pub fn new<'a>(ns: &'a str, db: &'a str, tb: &'a str, fd: &'a str) -> Fd<'a> {
	Fd::new(ns, db, tb, fd)
}

pub fn prefix(ns: &str, db: &str, tb: &str) -> Vec<u8> {
	let mut k = super::table::new(ns, db, tb).encode().unwrap();
	k.extend_from_slice(&[b'!', b'f', b'd', 0x00]);
	k
}

pub fn suffix(ns: &str, db: &str, tb: &str) -> Vec<u8> {
	let mut k = super::table::new(ns, db, tb).encode().unwrap();
	k.extend_from_slice(&[b'!', b'f', b'd', 0xff]);
	k
}

impl<'a> Fd<'a> {
	pub fn new(ns: &'a str, db: &'a str, tb: &'a str, fd: &'a str) -> Self {
		Self {
			__: b'/',
			_a: b'*',
			ns,
			_b: b'*',
			db,
			_c: b'*',
			tb,
			_d: b'!',
			_e: b'f',
			_f: b'd',
			fd,
		}
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn key() {
		use super::*;
		#[rustfmt::skip]
		let val = Fd::new(
			"test",
			"test",
			"test",
			"test",
		);
		let enc = Fd::encode(&val).unwrap();
		let dec = Fd::decode(&enc).unwrap();
		assert_eq!(val, dec);
	}
}
