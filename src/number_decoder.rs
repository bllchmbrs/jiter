use num_bigint::BigInt;
use num_traits::cast::ToPrimitive;
use std::fmt;
use std::marker::PhantomData;
use std::ops::Range;

use crate::{JsonError, JsonResult};

pub trait AbstractNumberDecoder {
    type Output;

    fn decode(data: &[u8], index: usize, first: u8) -> JsonResult<(Self::Output, usize)>;
}

pub trait AbstractNumber: fmt::Debug + PartialEq + Sized {
    fn new(digit: u8) -> Self;
    fn take_one(first: u8) -> JsonResult<Self>;

    fn add_digit(&mut self, digit: &u8) -> JsonResult<()>;

    fn apply_decimal(self, data: &[u8], index: usize, positive: bool) -> JsonResult<(Self, usize)>;

    fn apply_exponential(self, exponent: i32, positive: bool) -> JsonResult<Self>;

    fn negate(&mut self);
}

#[derive(Debug, Clone, PartialEq)]
pub enum NumberInt {
    Int(i64),
    BigInt(BigInt),
    // zero is special since a leading zero is only allowed on it's own or before "." or "e/E"
    Zero,
}

impl From<NumberInt> for f64 {
    fn from(num: NumberInt) -> Self {
        match num {
            NumberInt::Int(int) => int as f64,
            NumberInt::BigInt(big_int) => match big_int.to_f64() {
                Some(f) => f,
                None => f64::NAN,
            },
            NumberInt::Zero => 0.0,
        }
    }
}

impl AbstractNumber for NumberInt {
    fn new(digit: u8) -> Self {
        Self::Int((digit & 0x0f) as i64)
    }

    fn take_one(first: u8) -> JsonResult<Self> {
        match first {
            b'0' => Ok(Self::Zero),
            b'1'..=b'9' => Ok(Self::new(first)),
            _ => Err(JsonError::InvalidNumber),
        }
    }

    fn add_digit(&mut self, digit: &u8) -> JsonResult<()> {
        let digit_int = digit & 0x0f;
        match self {
            Self::Int(int_64) => {
                if let Some(mult_10) = int_64.checked_mul(10) {
                    if let Some(add_digit) = mult_10.checked_add((digit_int) as i64) {
                        *int_64 = add_digit;
                    } else {
                        let mut big_int: BigInt = mult_10.into();
                        big_int += digit_int;
                        *self = Self::BigInt(big_int);
                    }
                } else {
                    let mut big_int: BigInt = (*int_64).into();
                    big_int *= 10;
                    big_int += digit_int;
                    *self = Self::BigInt(big_int);
                }
            }
            Self::BigInt(ref mut big_int) => {
                *big_int *= 10;
                *big_int += digit_int;
            }
            Self::Zero => return Err(JsonError::InvalidNumber),
        }
        Ok(())
    }

    fn apply_decimal(self, _data: &[u8], _index: usize, _positive: bool) -> JsonResult<(Self, usize)> {
        Err(JsonError::FloatExpectingInt)
    }

    fn apply_exponential(self, _exponent: i32, _positive: bool) -> JsonResult<Self> {
        Err(JsonError::FloatExpectingInt)
    }

    fn negate(&mut self) {
        match self {
            Self::Int(int) => *int = -*int,
            Self::BigInt(big_int) => {
                *big_int *= -1;
            }
            Self::Zero => *self = Self::Int(-0),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum NumberAny {
    Int(NumberInt),
    Float(f64),
}

impl From<NumberAny> for f64 {
    fn from(num: NumberAny) -> Self {
        match num {
            NumberAny::Int(int) => int.into(),
            NumberAny::Float(float) => float,
        }
    }
}

impl AbstractNumber for NumberAny {
    fn new(digit: u8) -> Self {
        Self::Int(NumberInt::new(digit))
    }

    fn take_one(first: u8) -> JsonResult<Self> {
        NumberInt::take_one(first).map(Self::Int)
    }

    fn add_digit(&mut self, digit: &u8) -> JsonResult<()> {
        match self {
            Self::Int(int) => int.add_digit(digit),
            Self::Float(_) => Err(JsonError::InvalidNumber),
        }
    }

    fn apply_decimal(self, data: &[u8], mut index: usize, positive: bool) -> JsonResult<(Self, usize)> {
        let mut result: f64 = match self {
            Self::Int(int) => int.into(),
            Self::Float(_) => return Err(JsonError::InvalidNumber),
        };

        index += 1;
        let first = match data.get(index) {
            Some(v) if v.is_ascii_digit() => v,
            Some(_) => return Err(JsonError::InvalidNumber),
            None => return Err(JsonError::UnexpectedEnd),
        };
        result += (first & 0x0f) as f64 / 10.0;
        let mut div = 100.0;

        index += 1;
        while let Some(next) = data.get(index) {
            match next {
                b'0'..=b'9' => {
                    result += (next & 0x0f) as f64 / div;
                    div *= 10_f64;
                }
                b'e' | b'E' => {
                    let e = Exponent::decode(data, index)?;
                    let num = Self::Float(result).apply_exponential(e.0.value, positive)?;
                    return Ok((num, e.1));
                }
                _ => break,
            }
            index += 1;
        }

        let v = if positive { result } else { -result };
        Ok((Self::Float(v), index))
    }

    fn apply_exponential(self, exponent: i32, positive: bool) -> JsonResult<Self> {
        if self == Self::Int(NumberInt::Zero) {
            Ok(Self::Float(0.0))
        } else if exponent == i32::MAX {
            if positive {
                Ok(Self::Float(f64::INFINITY))
            } else {
                Ok(Self::Float(f64::NEG_INFINITY))
            }
        } else if exponent == i32::MIN {
            Ok(Self::Float(0.0))
        } else {
            let mut f: f64 = match self {
                Self::Int(int) => int.into(),
                Self::Float(float) => float,
            };
            f *= 10_f64.powi(exponent);
            if !positive {
                f = -f;
            }
            Ok(Self::Float(f))
        }
    }

    fn negate(&mut self) {
        match self {
            Self::Int(int) => int.negate(),
            Self::Float(f) => {
                *f = -*f;
            }
        }
    }
}

pub struct NumberDecoder<Num: AbstractNumber> {
    phantom: PhantomData<Num>,
}

impl<Num: AbstractNumber> AbstractNumberDecoder for NumberDecoder<Num> {
    type Output = Num;

    fn decode(data: &[u8], mut index: usize, first: u8) -> JsonResult<(Self::Output, usize)> {
        let positive = first != b'-';
        let first_num: u8 = if positive {
            first
        } else {
            // we started with a minus sign, so the first digit is at index + 1
            index += 1;
            *data.get(index).ok_or(JsonError::UnexpectedEnd)?
        };
        let mut num = Num::take_one(first_num)?;
        index += 1;
        while let Some(next) = data.get(index) {
            match next {
                b'0'..=b'9' => num.add_digit(next)?,
                b'.' => return num.apply_decimal(data, index, positive),
                b'e' | b'E' => {
                    let e = Exponent::decode(data, index)?;
                    num = num.apply_exponential(e.0.value, positive)?;
                    return Ok((num, e.1));
                }
                _ => break,
            }
            index += 1;
        }
        if positive {
            Ok((num, index))
        } else {
            num.negate();
            Ok((num, index))
        }
    }
}

#[derive(Debug)]
pub struct Exponent {
    value: i32,
}

impl Exponent {
    fn new(digit: u8) -> Self {
        Self {
            value: (digit & 0x0f) as i32,
        }
    }

    fn take_one(data: &[u8], index: usize) -> JsonResult<Self> {
        match data.get(index) {
            Some(digit) if digit.is_ascii_digit() => Ok(Self::new(*digit)),
            Some(_) => Err(JsonError::InvalidNumber),
            None => Err(JsonError::UnexpectedEnd),
        }
    }

    fn decode(data: &[u8], mut index: usize) -> JsonResult<(Self, usize)> {
        index += 1;
        let mut positive = true;

        let mut exp = match data.get(index) {
            Some(b'-') => {
                index += 1;
                positive = false;
                Self::take_one(data, index)?
            }
            Some(b'+') => {
                index += 1;
                Self::take_one(data, index)?
            }
            Some(digit) if digit.is_ascii_digit() => Self::new(*digit),
            Some(_) => return Err(JsonError::InvalidNumber),
            None => return Err(JsonError::UnexpectedEnd),
        };

        index += 1;
        while let Some(next) = data.get(index) {
            match next {
                b'0'..=b'9' => {
                    exp.value = match exp.value.checked_mul(10) {
                        Some(i) => i,
                        None => return Self::consume_rest(data, index, positive),
                    };
                    exp.value = match exp.value.checked_add((next & 0x0f) as i32) {
                        Some(i) => i,
                        None => return Self::consume_rest(data, index, positive),
                    };
                }
                _ => break,
            }
            index += 1;
        }

        if positive {
            Ok((exp, index))
        } else {
            exp.value = -exp.value;
            Ok((exp, index))
        }
    }

    fn consume_rest(data: &[u8], mut index: usize, positive: bool) -> JsonResult<(Self, usize)> {
        index += 1;
        while let Some(next) = data.get(index) {
            match next {
                b'0'..=b'9' => {
                    index += 1;
                }
                _ => break,
            }
        }

        let value = if positive { i32::MAX } else { i32::MIN };
        Ok((Self { value }, index))
    }
}

// TODO do we really need this, could we make it an impl of Number instead?
pub struct NumberDecoderRange;

impl AbstractNumberDecoder for NumberDecoderRange {
    type Output = Range<usize>;

    fn decode(data: &[u8], mut index: usize, first: u8) -> JsonResult<(Self::Output, usize)> {
        let start = index;
        let positive = first != b'-';
        if !positive {
            // we started with a minus sign, so the first digit is at index + 1
            index += 1;
        };
        // TODO special logic on zero
        match data.get(index) {
            Some(digit) if digit.is_ascii_digit() => (),
            Some(_) => return Err(JsonError::InvalidNumber),
            None => return Err(JsonError::UnexpectedEnd),
        };
        index += 1;
        while let Some(next) = data.get(index) {
            match next {
                // TODO proper logic related to leading zeros
                b'0'..=b'9' => (),
                b'.' => {
                    let end = numeric_range(data, index)?;
                    return Ok((start..end, end));
                }
                b'e' | b'E' => {
                    let end = exponent_range(data, index)?;
                    return Ok((start..end, end));
                }
                _ => break,
            }
            index += 1;
        }

        Ok((start..index, index))
    }
}

fn exponent_range(data: &[u8], mut index: usize) -> JsonResult<usize> {
    index += 1;

    match data.get(index) {
        Some(b'-') | Some(b'+') => {
            index += 1;
        }
        Some(v) if v.is_ascii_digit() => (),
        Some(_) => return Err(JsonError::InvalidNumber),
        None => return Err(JsonError::UnexpectedEnd),
    };
    numeric_range(data, index)
}

fn numeric_range(data: &[u8], mut index: usize) -> JsonResult<usize> {
    index += 1;

    match data.get(index) {
        Some(v) if v.is_ascii_digit() => (),
        Some(_) => return Err(JsonError::InvalidNumber),
        None => return Err(JsonError::UnexpectedEnd),
    };
    index += 1;

    while let Some(next) = data.get(index) {
        match next {
            b'0'..=b'9' => (),
            _ => break,
        }
        index += 1;
    }

    Ok(index)
}