//! Synthetic PDFs for the tests, made in-process with pdfcraft.

use std::path::PathBuf;
use std::sync::Arc;

use pdfcraft_cos::{Algorithm, NewEncryption};

/// A US Letter document (612 × 792 pt) of `pages` identical pages reading "Hello from PdfCraft".
pub(crate) fn letter(title: &str, pages: usize) -> Vec<u8> {
    let session = pdfcraft_engine::Session::new();
    let one = session.create_from_text(title, "Hello from PdfCraft").unwrap();
    if pages == 1 {
        return one.to_vec();
    }
    let sources: Vec<(String, Arc<Vec<u8>>)> = (0..pages).map(|i| (format!("page{i}"), one.clone())).collect();
    session.combine(&sources).unwrap().to_vec()
}

/// A one-page 200×300 pt PDF reading "Page 1".
fn small_page() -> Vec<u8> {
    let content = "BT /F1 24 Tf 20 150 Td (Page 1) Tj ET";
    let stream = format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len());
    let objs: [&[u8]; 5] = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 200 300] >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
        stream.as_bytes(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

pub(crate) const ALGORITHMS: [Algorithm; 4] = [Algorithm::Rc4_40, Algorithm::Rc4_128, Algorithm::Aes128, Algorithm::Aes256];

/// The 200×300 pt page protected with user password "user" and owner password "owner",
/// allowing nothing.
pub(crate) fn protected(algorithm: Algorithm) -> Vec<u8> {
    let mut doc = pdfcraft_cos::Document::open(Arc::new(small_page())).unwrap();
    doc.set_encryption(&NewEncryption {
        algorithm,
        user_password: "user",
        owner_password: "owner",
        permissions: 0,
        encrypt_metadata: true,
        seed: [3; 32],
    })
    .unwrap();
    pdfcraft_cos::write_full(&doc, &Default::default()).unwrap()
}

/// A dynamic XFA form whose template lays out into two 200×300 pt pages, while the file carries
/// one US Letter placeholder page. Written to a directory of the test's own.
pub(crate) fn xfa_form(test: &str) -> PathBuf {
    let xdp = r#"<xdp:xdp xmlns:xdp="http://ns.adobe.com/xdp/">
<template xmlns="http://www.xfa.org/schema/xfa-template/3.3/">
<subform name="form" layout="tb">
 <pageSet><pageArea name="page"><occur max="-1"/>
  <contentArea x="20pt" y="20pt" w="160pt" h="260pt"/>
  <medium short="200pt" long="300pt"/>
 </pageArea></pageSet>
 <draw w="160pt" h="20pt"><value><text>First XFA page</text></value></draw>
 <field name="name" w="160pt" h="20pt"><ui><textEdit/></ui><value><text>Ada</text></value></field>
 <subform name="last" layout="tb"><breakBefore targetType="pageArea"/>
  <draw w="160pt" h="20pt"><value><text>Second XFA page</text></value></draw>
 </subform>
</subform></template></xdp:xdp>"#;
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /NeedsRendering true /AcroForm << /XFA 5 0 R /Fields [] >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>".into(),
        "<< /Length 0 >>\nstream\n\nendstream".into(),
        format!("<< /Length {} >>\nstream\n{xdp}\nendstream", xdp.len()),
    ];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    let dir = std::env::temp_dir().join(format!("clview-xfa-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("form.pdf");
    std::fs::write(&path, bytes).unwrap();
    path
}
