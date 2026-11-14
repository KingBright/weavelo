//! PS5 Takion Cauchy Reed-Solomon FEC over GF(2^8).

use thiserror::Error;

use crate::gf256;

#[derive(Debug, Error)]
pub enum FecError {
    #[error("too many erasures ({erasures}) for {m} parity units")]
    TooManyErasures { erasures: usize, m: usize },
    #[error("invalid FEC geometry k={k}, m={m}")]
    InvalidGeometry { k: usize, m: usize },
    #[error("invalid erasure index {index} for total units {total}")]
    InvalidErasureIndex { index: usize, total: usize },
    #[error("duplicate erasure index {index}")]
    DuplicateErasure { index: usize },
    #[error("frame buffer too small for FEC layout")]
    BufferTooSmall,
    #[error("singular FEC decoding matrix")]
    SingularMatrix,
}

/// Builds Jerasure's `cauchy_original_coding_matrix(k, m, 8)` (`m x k` row-major).
pub fn cauchy_original_coding_matrix(k: usize, m: usize) -> Result<Vec<u8>, FecError> {
    if k == 0 || m == 0 || k + m > 256 {
        return Err(FecError::InvalidGeometry { k, m });
    }
    let mut mat = vec![0u8; m * k];
    for i in 0..m {
        for j in 0..k {
            let denom = (i as u8) ^ ((m + j) as u8);
            mat[i * k + j] = gf256::div(1, denom);
        }
    }
    Ok(mat)
}

/// Encodes `m` parity units at slots `k..k+m` in `frame_buf`.
pub fn fec_encode(
    frame_buf: &mut [u8],
    unit_size: usize,
    stride: usize,
    k: usize,
    m: usize,
) -> Result<(), FecError> {
    validate_buffer(frame_buf.len(), unit_size, stride, k, m)?;
    let mat = cauchy_original_coding_matrix(k, m)?;
    for i in 0..m {
        let p_off = (k + i) * stride;
        frame_buf[p_off..p_off + unit_size].fill(0);
        for j in 0..k {
            let s_off = j * stride;
            let (left, right) = frame_buf.split_at_mut(p_off);
            let src = &left[s_off..s_off + unit_size];
            let dst = &mut right[..unit_size];
            gf256::mul_add_slice(dst, src, mat[i * k + j]);
        }
    }
    Ok(())
}

/// Recovers erased source units (`0..k`) using surviving data and Cauchy parity units.
pub fn fec_decode(
    frame_buf: &mut [u8],
    unit_size: usize,
    stride: usize,
    k: usize,
    m: usize,
    erasures: &[usize],
) -> Result<(), FecError> {
    validate_buffer(frame_buf.len(), unit_size, stride, k, m)?;
    if erasures.len() > m {
        return Err(FecError::TooManyErasures {
            erasures: erasures.len(),
            m,
        });
    }

    let total = k + m;
    let mut is_erased = vec![false; total];
    for &e in erasures {
        if e >= total {
            return Err(FecError::InvalidErasureIndex { index: e, total });
        }
        if is_erased[e] {
            return Err(FecError::DuplicateErasure { index: e });
        }
        is_erased[e] = true;
    }

    let erased_data: Vec<usize> = (0..k).filter(|&i| is_erased[i]).collect();
    if erased_data.is_empty() {
        return Ok(());
    }

    let mat = cauchy_original_coding_matrix(k, m)?;
    let survivors: Vec<usize> = (0..total).filter(|&i| !is_erased[i]).take(k).collect();
    if survivors.len() < k {
        return Err(FecError::TooManyErasures {
            erasures: erasures.len(),
            m,
        });
    }

    // Build `k x k` matrix whose rows are the surviving units expressed in data basis `0..k`.
    let mut sub = vec![0u8; k * k];
    for (row_idx, &unit) in survivors.iter().enumerate() {
        if unit < k {
            sub[row_idx * k + unit] = 1;
        } else {
            let parity_idx = unit - k;
            sub[row_idx * k..(row_idx + 1) * k]
                .copy_from_slice(&mat[parity_idx * k..(parity_idx + 1) * k]);
        }
    }

    if !gf256::invert_matrix(&mut sub, k) {
        return Err(FecError::SingularMatrix);
    }

    // Recover each missing data unit into a temporary buffer so surviving units remain intact.
    let mut recovered = Vec::with_capacity(erased_data.len());
    for &d in &erased_data {
        let mut out = vec![0u8; unit_size];
        for (col, &surv) in survivors.iter().enumerate() {
            let coeff = sub[d * k + col];
            let s_off = surv * stride;
            gf256::mul_add_slice(&mut out, &frame_buf[s_off..s_off + unit_size], coeff);
        }
        recovered.push((d, out));
    }

    for (d, bytes) in recovered {
        let d_off = d * stride;
        frame_buf[d_off..d_off + unit_size].copy_from_slice(&bytes);
    }

    Ok(())
}

fn validate_buffer(
    buf_len: usize,
    unit_size: usize,
    stride: usize,
    k: usize,
    m: usize,
) -> Result<(), FecError> {
    if k == 0 || m == 0 || k + m > 256 {
        return Err(FecError::InvalidGeometry { k, m });
    }
    if stride < unit_size {
        return Err(FecError::BufferTooSmall);
    }
    let required = (k + m)
        .checked_mul(stride)
        .ok_or(FecError::BufferTooSmall)?;
    if buf_len < required {
        return Err(FecError::BufferTooSmall);
    }
    Ok(())
}
