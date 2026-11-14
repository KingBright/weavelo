//! Galois Field GF(2^8) arithmetic and matrix inversion matching Jerasure (`0x11d`).

use std::sync::LazyLock;

const PRIMITIVE_POLY: u16 = 0x11d;

struct GfTables {
    exp: [u8; 512],
    log: [u8; 256],
    mul: [[u8; 256]; 256],
}

static TABLES: LazyLock<GfTables> = LazyLock::new(|| {
    let mut exp = [0u8; 512];
    let mut log = [0u8; 256];
    let mut x: u16 = 1;
    for i in 0..255usize {
        exp[i] = x as u8;
        exp[i + 255] = x as u8;
        log[x as usize] = i as u8;
        x <<= 1;
        if (x & 0x100) != 0 {
            x ^= PRIMITIVE_POLY;
        }
    }

    let mut mul = [[0u8; 256]; 256];
    for a in 1..256usize {
        for b in 1..256usize {
            let sum = log[a] as usize + log[b] as usize;
            mul[a][b] = exp[sum];
        }
    }

    GfTables { exp, log, mul }
});

#[inline]
pub fn mul(a: u8, b: u8) -> u8 {
    TABLES.mul[a as usize][b as usize]
}

#[inline]
pub fn div(a: u8, b: u8) -> u8 {
    assert_ne!(b, 0, "division by zero in GF(2^8)");
    if a == 0 {
        return 0;
    }
    let la = TABLES.log[a as usize] as i16;
    let lb = TABLES.log[b as usize] as i16;
    let idx = (la - lb).rem_euclid(255) as usize;
    TABLES.exp[idx]
}

#[inline]
pub fn inv(a: u8) -> u8 {
    div(1, a)
}

/// Computes `dst[i] ^= coeff * src[i]` across matching slice lengths.
pub fn mul_add_slice(dst: &mut [u8], src: &[u8], coeff: u8) {
    assert_eq!(dst.len(), src.len());
    if coeff == 0 {
        return;
    }
    if coeff == 1 {
        for (d, s) in dst.iter_mut().zip(src.iter()) {
            *d ^= *s;
        }
        return;
    }
    let row = &TABLES.mul[coeff as usize];
    for (d, s) in dst.iter_mut().zip(src.iter()) {
        *d ^= row[*s as usize];
    }
}

/// Inverts an `n x n` matrix over GF(2^8) in-place using Gauss-Jordan elimination.
pub fn invert_matrix(mat: &mut [u8], n: usize) -> bool {
    assert_eq!(mat.len(), n * n);
    let mut inv_mat = vec![0u8; n * n];
    for i in 0..n {
        inv_mat[i * n + i] = 1;
    }

    for col in 0..n {
        let pivot_row = (col..n).find(|&r| mat[r * n + col] != 0);
        let Some(pivot) = pivot_row else {
            return false;
        };
        if pivot != col {
            for c in 0..n {
                mat.swap(col * n + c, pivot * n + c);
                inv_mat.swap(col * n + c, pivot * n + c);
            }
        }

        let diag = mat[col * n + col];
        if diag != 1 {
            let factor = inv(diag);
            for c in 0..n {
                mat[col * n + c] = mul(mat[col * n + c], factor);
                inv_mat[col * n + c] = mul(inv_mat[col * n + c], factor);
            }
        }

        for r in 0..n {
            if r == col {
                continue;
            }
            let coeff = mat[r * n + col];
            if coeff == 0 {
                continue;
            }
            for c in 0..n {
                mat[r * n + c] ^= mul(coeff, mat[col * n + c]);
                inv_mat[r * n + c] ^= mul(coeff, inv_mat[col * n + c]);
            }
        }
    }

    mat.copy_from_slice(&inv_mat);
    true
}
