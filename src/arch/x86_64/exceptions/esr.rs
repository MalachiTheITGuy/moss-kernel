//! x86_64 Exception Syndrome Register (ESR) decoding.
//!
//! For x86_64, the "ESR" equivalent is the error code pushed on the
//! stack for certain exceptions (GP, PF, SS, NP, TS, AC, DF).

/// Decode an x86_64 exception error code into a human-readable string.
pub fn decode_error_code(vector: usize, error_code: u64) -> &'static str {
    match vector {
        8 => "Double Fault (always 0)",
        10 => "Invalid TSS",
        11 => "Segment Not Present",
        12 => "Stack Segment Fault",
        13 => "General Protection Fault",
        14 => {
            let mut desc = "Page Fault";
            let mut buf = [0u8; 128];
            let mut pos = 0;

            if error_code & 0x01 != 0 {
                // Could use write! macro but let's keep it simple.
            }

            if error_code & 0x02 != 0 {
                // Write access.
            }

            if error_code & 0x04 != 0 {
                // User-mode.
            }

            if error_code & 0x08 != 0 {
                // Reserved bit violation.
            }

            if error_code & 0x10 != 0 {
                // Instruction fetch.
            }

            desc
        }
        17 => "Alignment Check",
        30 => "Security Exception",
        _ => "Unknown error code",
    }
}

/// Returns true if the given vector pushes an error code onto the stack.
pub fn has_error_code(vector: usize) -> bool {
    matches!(vector, 8 | 10 | 11 | 12 | 13 | 14 | 17 | 21 | 29 | 30)
}
