use anyhow::{anyhow, bail, Result};

// RFC 6962 §3.4 — MerkleTreeLeaf with TimestampedEntry.
//
// struct {
//     Version version;                   // u8 (always 0 for v1)
//     MerkleLeafType leaf_type;          // u8 (always 0 for timestamped_entry)
//     select (leaf_type) {
//         case timestamped_entry: TimestampedEntry timestamped_entry;
//     }
// } MerkleTreeLeaf;
//
// struct {
//     uint64 timestamp;
//     LogEntryType entry_type;           // u16 (0=x509_entry, 1=precert_entry)
//     select(entry_type) {
//         case x509_entry:    ASN.1Cert signed_entry;       // <1..2^24-1> length-prefixed
//         case precert_entry: PreCert    signed_entry;      // = { sha256(issuer pubkey) | tbs_certificate }
//     };
//     CtExtensions extensions;           // <0..2^16-1>
// } TimestampedEntry;

#[derive(Debug)]
pub struct DecodedEntry {
    pub timestamp_ms: u64,
    pub kind: EntryKind,
    pub precert_der: Option<Vec<u8>>, // populated from extra_data for precert entries
}

#[derive(Debug)]
pub enum EntryKind {
    Precert {
        issuer_key_hash: [u8; 32],
        tbs: Vec<u8>,
    },
    FinalCert {
        der: Vec<u8>,
    },
}

pub fn decode_leaf(leaf_input: &[u8], extra_data: &[u8]) -> Result<DecodedEntry> {
    let mut r = Reader::new(leaf_input);
    let version = r.u8()?;
    if version != 0 {
        bail!("unsupported MerkleTreeLeaf version: {version}");
    }
    let leaf_type = r.u8()?;
    if leaf_type != 0 {
        bail!("unsupported leaf_type: {leaf_type}");
    }
    let timestamp_ms = r.u64()?;
    let entry_type = r.u16()?;

    let (kind, precert_der) = match entry_type {
        0 => {
            // x509_entry: opaque ASN.1Cert<1..2^24-1>
            let der = r.var_bytes(3)?;
            (EntryKind::FinalCert { der }, None)
        }
        1 => {
            // precert_entry: { issuer_key_hash[32], tbs_certificate<1..2^24-1> }
            let mut ikh = [0u8; 32];
            r.read_exact(&mut ikh)?;
            let tbs = r.var_bytes(3)?;
            // Pull the actual precert DER out of extra_data:
            // PrecertChainEntry { ASN.1Cert pre_certificate; ASN.1Cert chain<0..2^24-1>; }
            let mut er = Reader::new(extra_data);
            let der = er.var_bytes(3)?;
            (
                EntryKind::Precert {
                    issuer_key_hash: ikh,
                    tbs,
                },
                Some(der),
            )
        }
        n => bail!("unknown entry_type {n}"),
    };

    // skip extensions
    let _ext = r.var_bytes(2)?;

    Ok(DecodedEntry {
        timestamp_ms,
        kind,
        precert_der,
    })
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn read_exact(&mut self, dst: &mut [u8]) -> Result<()> {
        if self.pos + dst.len() > self.buf.len() {
            return Err(anyhow!("short read"));
        }
        dst.copy_from_slice(&self.buf[self.pos..self.pos + dst.len()]);
        self.pos += dst.len();
        Ok(())
    }
    fn u8(&mut self) -> Result<u8> {
        let mut b = [0u8; 1];
        self.read_exact(&mut b)?;
        Ok(b[0])
    }
    fn u16(&mut self) -> Result<u16> {
        let mut b = [0u8; 2];
        self.read_exact(&mut b)?;
        Ok(u16::from_be_bytes(b))
    }
    fn u64(&mut self) -> Result<u64> {
        let mut b = [0u8; 8];
        self.read_exact(&mut b)?;
        Ok(u64::from_be_bytes(b))
    }
    fn var_bytes(&mut self, len_bytes: usize) -> Result<Vec<u8>> {
        let mut len: u64 = 0;
        for _ in 0..len_bytes {
            len = (len << 8) | self.u8()? as u64;
        }
        let len = len as usize;
        if self.pos + len > self.buf.len() {
            return Err(anyhow!("short var-bytes read"));
        }
        let v = self.buf[self.pos..self.pos + len].to_vec();
        self.pos += len;
        Ok(v)
    }
}
