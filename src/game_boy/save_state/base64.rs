use std::slice::SliceIndex;

use base64::engine::general_purpose::STANDARD as BASE64;
use nanoserde::{DeRon, DeRonErr, DeRonState, SerRon, SerRonState};

pub struct Base64Bytes(pub Vec<u8>);

impl std::ops::Deref for Base64Bytes {
    type Target = Vec<u8>;
    fn deref(&self) -> &Vec<u8> {
        &self.0
    }
}

impl<I: SliceIndex<[u8]>> std::ops::Index<I> for Base64Bytes {
    type Output = I::Output;
    fn index(&self, index: I) -> &Self::Output {
        &self.0[index]
    }
}

impl Base64Bytes {
    pub fn from_banks<const N: usize>(banks: &[[u8; N]]) -> Self {
        Self(banks.iter().flatten().copied().collect())
    }

    pub fn into_banks<const N: usize>(&self, num_banks: usize) -> Vec<[u8; N]> {
        let mut result = vec![[0u8; N]; num_banks];
        for (i, bank) in result.iter_mut().enumerate() {
            let offset = i * N;
            if offset < self.len() {
                let len = (self.len() - offset).min(N);
                bank[..len].copy_from_slice(&self[offset..offset + len]);
            }
        }
        result
    }
}

impl SerRon for Base64Bytes {
    fn ser_ron(&self, _indent_level: usize, state: &mut SerRonState) {
        let encoded = base64::Engine::encode(&BASE64, &self.0);
        state.out.push('"');
        state.out.push_str(&encoded);
        state.out.push('"');
    }
}

impl DeRon for Base64Bytes {
    fn de_ron(state: &mut DeRonState, input: &mut std::str::Chars<'_>) -> Result<Self, DeRonErr> {
        let s = String::de_ron(state, input)?;
        let bytes =
            base64::Engine::decode(&BASE64, &s).map_err(|e| state.err_parse(&e.to_string()))?;
        Ok(Self(bytes))
    }
}

pub struct Base64Array<const N: usize>(pub [u8; N]);

impl<const N: usize> SerRon for Base64Array<N> {
    fn ser_ron(&self, _indent_level: usize, state: &mut SerRonState) {
        let encoded = base64::Engine::encode(&BASE64, &self.0);
        state.out.push('"');
        state.out.push_str(&encoded);
        state.out.push('"');
    }
}

impl<const N: usize> DeRon for Base64Array<N> {
    fn de_ron(state: &mut DeRonState, input: &mut std::str::Chars<'_>) -> Result<Self, DeRonErr> {
        let s = String::de_ron(state, input)?;
        let bytes =
            base64::Engine::decode(&BASE64, &s).map_err(|e| state.err_parse(&e.to_string()))?;
        let arr: [u8; N] = bytes
            .try_into()
            .map_err(|_| state.err_parse(&format!("expected {N} bytes")))?;
        Ok(Self(arr))
    }
}
