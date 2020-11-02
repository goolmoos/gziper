use std::io::Write;

use crate::huffman;
mod lempel_ziv;
mod block_splitter;
use block_splitter::Block;

pub enum Token {
	Literal(u8),
	Repeat(u32, u32),
}

pub fn deflate<T: Write>(file: &[u8], out: &mut T) {
	let tokens = lempel_ziv::lempel_ziv(file);
	let blocks = block_splitter::block_split(&tokens);
	let mut writer = DeflateWriter::new(out);

	for (i, block) in blocks.iter().enumerate() {
		let is_last = i == blocks.len() - 1;
		match block {
			Block::FixedCodes { tokens } => {
				writer.new_fixed_codes_block(is_last);
				tokens.iter().for_each(|t| writer.write(t));
			}
			Block::DynamicCodes { tokens, literal_code_lens, distance_code_lens } => {
				writer.new_dynamic_codes_block(is_last, literal_code_lens, distance_code_lens);
				tokens.iter().for_each(|t| writer.write(t));
			}
		}
	}
}

struct DeflateWriter<'a, T: Write> {
	out: &'a mut T,
	curr_bytes: u32,
	curr_full_bits: u8,
	literal_tree: huffman::Tree,
	distance_tree: huffman::Tree,
	in_block: bool,
}

impl<'a, T: Write> DeflateWriter<'a, T> {
	fn new(out: &'a mut T) -> DeflateWriter<'a, T> {
		DeflateWriter {
			out,
			curr_bytes: 0,
			curr_full_bits: 0,
			literal_tree: vec![],
			distance_tree: vec![],
			in_block: false,
		}
	}

	fn write_bits(&mut self, bits: u32, len: u8) {
		// packs from LSB to MSB
		// 16 bit max
		self.curr_bytes |= bits << self.curr_full_bits;
		self.curr_full_bits += len;
		while self.curr_full_bits >= 8 {
			self.out.write_all(&[(self.curr_bytes & 0xFF) as u8]).unwrap();
			self.curr_bytes >>= 8;
			self.curr_full_bits -= 8;
		}
	}

	fn write(&mut self, token: &Token) {
		match token {
			Token::Literal(value) => {
				let huffman_code = self.literal_tree[*value as usize];
				self.write_bits(huffman_code.code, huffman_code.length);
			}
			Token::Repeat(len, dist) => {
				for (len_start, len_end, extra_bits, code) in &LEN_TO_CODE {
					if len < len_end {
						let huffman_code = self.literal_tree[*code as usize];
						self.write_bits(huffman_code.code, huffman_code.length);
						self.write_bits(len - len_start, *extra_bits);
						break;
					}
				}
				for (dist_start, dist_end, extra_bits, code) in &DIST_TO_CODE {
					if dist < dist_end {
						let huffman_code = self.distance_tree[*code as usize];
						self.write_bits(huffman_code.code, huffman_code.length);
						self.write_bits(dist - dist_start, *extra_bits);
						break;
					}
				}
			}
		};
	}

	fn new_fixed_codes_block(&mut self, is_final: bool) {
		if self.in_block {
			// end of block
			let huffman_code = self.literal_tree[256];
			self.write_bits(huffman_code.code, huffman_code.length);
		}
		self.in_block = true;
		self.write_bits(if is_final {1} else {0}, 1);
		self.write_bits(1, 1);
		self.write_bits(0, 1);
		self.literal_tree = huffman::calc_codes(&huffman::LITERAL_FIXED_CODES);
		self.distance_tree = huffman::calc_codes(&huffman::DISTANCE_FIXED_CODES);
	}

	fn new_dynamic_codes_block(&mut self, is_final: bool, literal_code_lens: &[u8], distance_code_lens: &[u8]) {
		if self.in_block {
			// end of block
			let huffman_code = self.literal_tree[256];
			self.write_bits(huffman_code.code, huffman_code.length);
		}
		self.in_block = true;
		self.write_bits(if is_final {1} else {0}, 1);
		self.write_bits(0, 1);
		self.write_bits(1, 1);
		// encode tree
		self.write_bits(286 - 257, 5); // HLIT
		self.write_bits(30 - 1, 5); // HDIST
		self.write_bits(19 - 4, 4); // HCLEN
		let code_len_of_code_order: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
		let code_len_of_code: [u8; 19] = [4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0];
		let code_len_tree = huffman::calc_codes(&code_len_of_code);
		for i in 0..19 { // code lengths for the code length alphabet
			self.write_bits(code_len_of_code[code_len_of_code_order[i]] as u32, 3);
		}
		for i in 0..286 { // code lengths for the literal/length alphabet
			let huffman_code = code_len_tree[literal_code_lens[i] as usize];
			self.write_bits(huffman_code.code, huffman_code.length);
		}
		for i in 0..30 { // code lengths for the distance alphabet
			let huffman_code = code_len_tree[distance_code_lens[i] as usize];
			self.write_bits(huffman_code.code, huffman_code.length);
		}
		self.literal_tree = huffman::calc_codes(literal_code_lens);
		self.distance_tree = huffman::calc_codes(distance_code_lens);
	}
}

impl<'a, T: Write> Drop for DeflateWriter<'a, T> {
	fn drop(&mut self) {
		// end of block
		let huffman_code = self.literal_tree[256];
		self.write_bits(huffman_code.code, huffman_code.length);
		if self.curr_full_bits > 0 {
			self.out.write_all(&[(self.curr_bytes & 0xFF) as u8]).unwrap();
		}
	}
}

const LEN_TO_CODE: [(u32, u32, u8, u32); 29] = [
// (len start, len end, extra bits, code)
(3  , 4  , 0, 257),
(4  , 5  , 0, 258),
(5  , 6  , 0, 259),
(6  , 7  , 0, 260),
(7  , 8  , 0, 261),
(8  , 9  , 0, 262),
(9  , 10 , 0, 263),
(10 , 11 , 0, 264),
(11 , 13 , 1, 265),
(13 , 15 , 1, 266),
(15 , 17 , 1, 267),
(17 , 19 , 1, 268),
(19 , 23 , 2, 269),
(23 , 27 , 2, 270),
(27 , 31 , 2, 271),
(31 , 35 , 2, 272),
(35 , 43 , 3, 273),
(43 , 51 , 3, 274),
(51 , 59 , 3, 275),
(59 , 67 , 3, 276),
(67 , 83 , 4, 277),
(83 , 99 , 4, 278),
(99 , 115, 4, 279),
(115, 131, 4, 280),
(131, 163, 5, 281),
(163, 195, 5, 282),
(195, 227, 5, 283),
(227, 258, 5, 284),
(258, 259, 0, 285)
];

const DIST_TO_CODE: [(u32, u32, u8, u32); 30] = [
// (dist start, dist end, extra bits, code)
(1    , 2    , 0 , 0 ),
(2    , 3    , 0 , 1 ),
(3    , 4    , 0 , 2 ),
(4    , 5    , 0 , 3 ),
(5    , 7    , 1 , 4 ),
(7    , 9    , 1 , 5 ),
(9    , 13   , 2 , 6 ),
(13   , 17   , 2 , 7 ),
(17   , 25   , 3 , 8 ),
(25   , 33   , 3 , 9 ),
(33   , 49   , 4 , 10),
(49   , 65   , 4 , 11),
(65   , 97   , 5 , 12),
(97   , 129  , 5 , 13),
(129  , 193  , 6 , 14),
(193  , 257  , 6 , 15),
(257  , 385  , 7 , 16),
(385  , 513  , 7 , 17),
(513  , 769  , 8 , 18),
(769  , 1025 , 8 , 19),
(1025 , 1537 , 9 , 20),
(1537 , 2049 , 9 , 21),
(2049 , 3073 , 10, 22),
(3073 , 4097 , 10, 23),
(4097 , 6145 , 11, 24),
(6145 , 8193 , 11, 25),
(8193 , 12289, 12, 26),
(12289, 16385, 12, 27),
(16385, 24577, 13, 28),
(24577, 32769, 13, 29)
];
