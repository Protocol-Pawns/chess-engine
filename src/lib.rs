#![feature(coroutines, coroutine_trait)]
#![no_std]
#[macro_use]
extern crate alloc;
use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec::Vec,
};
use core::{
    convert::TryFrom,
    ops::{Coroutine, CoroutineState},
    pin::Pin,
};
use near_sdk::{
    borsh::{self, BorshDeserialize, BorshSerialize},
    serde::{Deserialize, Serialize},
};
use witgen::witgen;

mod board;
pub use board::{Board, BoardBuilder};

mod game;
pub use game::{Game, GameAction, GameError, GameOver};

mod square;
pub use square::{Square, EMPTY_SQUARE};

mod piece;
pub use piece::Piece;

mod position;
pub use position::*;

mod util;
pub use util::*;

pub const WHITE: Color = Color::White;
pub const BLACK: Color = Color::Black;

/// The result of a move being played on the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[allow(clippy::large_enum_variant)]
pub enum GameResult {
    /// The game is not finished, and the game is still in play.
    Continuing(Board),
    /// One player, the victor, checkmated the other.
    /// This stores the color of the winner.
    Victory(Color),
    /// The game is drawn. This can be a result of the current player
    /// having no legal moves and not being in check, or because
    /// both players have insufficient material on the board.
    ///
    /// Insufficient material consists of:
    /// 1. The player only has a king
    /// 2. The player only has a king and a knight
    /// 3. The player only has a king and two knights
    /// 4. The player only has a king and a bishop
    /// 5. The player only has a king and two bishops
    ///
    /// In a regular game of chess, threefold repetition also triggers
    /// a stalemate, but this engine does not have builtin support for
    /// threefold repetition detection yet.
    Stalemate,
    /// An illegal move was made. This can include many things,
    /// such as moving a piece through another piece, attempting
    /// to capture an allied piece, moving non-orthogonally or
    /// non-diagonally, or non-knight-like according the rules
    /// governing the movement of the piece. Additionally,
    /// moves that put the player in check, (for example, moving a pinned piece),
    /// are also illegal.
    IllegalMove(Move),
}

/// The color of a piece.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    BorshDeserialize,
    BorshSerialize,
    Deserialize,
    Serialize,
)]
#[serde(crate = "near_sdk::serde")]
#[witgen]
pub enum Color {
    White,
    Black,
}

impl core::fmt::Display for Color {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> Result<(), core::fmt::Error> {
        write!(
            f,
            "{}",
            match self {
                Self::White => "White",
                Self::Black => "Black",
            }
        )
    }
}

/// A color can be inverted using the `!` operator.
/// `!Color::White` becomes `Color::Black` and vice versa.
impl core::ops::Not for Color {
    type Output = Self;
    fn not(self) -> Self {
        match self {
            Self::White => Self::Black,
            Self::Black => Self::White,
        }
    }
}

/// A move that can be applied to a board.
/// When applied to a board, the board assumes that the move is
/// being applied for the current turn's player.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Move {
    /// If the current player is white, move the king to the C1 square, and the kingside rook to
    /// the D1 square. If the current player is black, however, move the king to the C8 square,
    /// and the kingside rook to the D8 square.
    ///
    /// Castling can only be performed if
    /// 1. The king has not moved at all since the game began
    /// 2. The respective rook (kingside or queenside) has also not moved
    /// 3. The square adjacent to the king on the respective side is not threatened by an enemy piece
    ///
    /// If all of these conditions are satisfied, castling is a legal move
    QueenSideCastle,
    /// If the current player is white, move the king to the G1 square, and the kingside rook to
    /// the F1 square. If the current player is black, however, move the king to the G8 square,
    /// and the kingside rook to the F8 square.
    KingSideCastle,
    /// Move a piece from one square to another, with optional promotion.
    /// This can allow the player to capture another piece, by
    /// simply moving a piece to the position of an enemy piece.
    ///
    /// Additionally, this can be used to [en-passant capture](https://en.wikipedia.org/wiki/En_passant),
    /// even though the en-passant square itself does not contain any capturable pieces.
    ///
    /// En-passant captures MUST be performed with a pawn, upon an enemy pawn
    /// that has just surpassed it by move two squares. An en-passant capture
    /// must also be performed the turn immediately after the enemy pawn surpasses
    /// the allied pawn. After the one turn a player has to en-passant capture, the
    /// en-passant square is forgotten and can no longer be used.
    Piece(Position, Position),
    Promotion(Position, Position, Piece),
    /// When played by another player, it awards victory to the other.
    Resign,
}

/// Try to parse a Move from a string.
///
/// Possible valid formats include:
/// - `"resign"`
/// - `"resigns"`
/// - `"castle queenside"`
/// - `"O-O-O"` (correct notation)
/// - `"o-o-o"` (incorrect notation, but will accept)
/// - `"0-0-0"` (incorrect notation, but will accept)
/// - `"castle kingside"`
/// - `"O-O"` (correct notation)
/// - `"o-o"` (incorrect notation, but will accept)
/// - `"0-0"` (incorrect notation, but will accept)
/// - `"e2e4"`
/// - `"e2 e4"`
/// - `"e2 to e4"`
///
/// Parsing a move such as `"knight to e4"` or `"Qxe4"` will NOT work.
impl TryFrom<String> for Move {
    type Error = String;

    fn try_from(repr: String) -> Result<Self, Self::Error> {
        let repr = repr.trim().to_string();

        Ok(match repr.as_str() {
            "resign" | "resigns" => Self::Resign,
            "queenside castle" | "castle queenside" | "O-O-O" | "0-0-0" | "o-o-o" => {
                Self::QueenSideCastle
            }
            "kingside castle" | "castle kingside" | "O-O" | "0-0" | "o-o" => Self::KingSideCastle,
            other => {
                let words = other.split_whitespace().collect::<Vec<&str>>();

                if words.len() == 1 && words[0].len() == 4 {
                    Self::Piece(
                        Position::pgn(&words[0][..2])?,
                        Position::pgn(&words[0][2..4])?,
                    )
                } else if words.len() == 2 {
                    Self::Piece(Position::pgn(words[0])?, Position::pgn(words[1])?)
                } else if words.len() == 3 && words[1] == "to" {
                    Self::Piece(Position::pgn(words[0])?, Position::pgn(words[2])?)
                } else if words.len() == 4 && words[1] == "to" {
                    let piece = Piece::try_from(words[3])?;
                    if piece.is_king() || piece.is_pawn() {
                        return Err(String::from("invalid promotion"));
                    }
                    Self::Promotion(Position::pgn(words[0])?, Position::pgn(words[2])?, piece)
                } else {
                    return Err(format!("invalid move format `{}`", other));
                }
            }
        })
    }
}

impl Move {
    /// Try to parse a Move from a string.
    ///
    /// Possible valid formats include:
    /// - `"resign"`
    /// - `"resigns"`
    /// - `"castle queenside"`
    /// - `"O-O-O"` (correct notation)
    /// - `"o-o-o"` (incorrect notation, but will accept)
    /// - `"0-0-0"` (incorrect notation, but will accept)
    /// - `"castle kingside"`
    /// - `"O-O"` (correct notation)
    /// - `"o-o"` (incorrect notation, but will accept)
    /// - `"0-0"` (incorrect notation, but will accept)
    /// - `"e2e4"`
    /// - `"e2 e4"`
    /// - `"e2 to e4"`
    ///
    /// Parsing a move such as `"knight to e4"` or `"Qxe4"` will NOT work.
    pub fn parse(repr: String) -> Result<Self, String> {
        Self::try_from(repr)
    }
}

impl core::fmt::Display for Move {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> Result<(), core::fmt::Error> {
        match self {
            // Move::EnPassant(from) => write!(f, "ep {}", from),
            Move::Piece(from, to) => write!(f, "{} to {}", from, to),
            Move::Promotion(from, to, piece) => {
                write!(f, "{} to {} {}", from, to, piece.get_name())
            }
            Move::KingSideCastle => write!(f, "O-O"),
            Move::QueenSideCastle => write!(f, "O-O-O"),
            Move::Resign => write!(f, "Resign"),
        }
    }
}

pub(crate) struct CoroutineIteratorAdapter<G>(Pin<Box<G>>);

impl<G> CoroutineIteratorAdapter<G>
where
    G: Coroutine<Return = ()>,
{
    fn new(gen: G) -> Self {
        Self(Box::pin(gen))
    }
}

impl<G> Iterator for CoroutineIteratorAdapter<G>
where
    G: Coroutine<Return = ()>,
{
    type Item = G::Yield;

    fn next(&mut self) -> Option<Self::Item> {
        match self.0.as_mut().resume(()) {
            CoroutineState::Yielded(x) => Some(x),
            CoroutineState::Complete(_) => None,
        }
    }
}
