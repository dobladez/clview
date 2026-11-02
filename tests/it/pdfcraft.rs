//! The PDF backend, `src/source/pdfcraft.rs`: what pdfcraft does for a PDF beyond drawing its
//! pages, and the rule that nothing else names it, so it can be swapped for another renderer.

use pdfcraft_cos::{Algorithm, NewEncryption};

use crate::fixtures::pdf;
use crate::view::{images, png_size, text, tmp, view};

/// A one-page 200 × 300 pt PDF reading "Page 1".
fn small_page() -> Vec<u8> {
    let content = "BT /F1 24 Tf 20 150 Td (Page 1) Tj ET";
    pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 200 300] >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /Font << /F1 3 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    ])
}

const ALGORITHMS: [Algorithm; 4] = [Algorithm::Rc4_40, Algorithm::Rc4_128, Algorithm::Aes128, Algorithm::Aes256];

/// The 200 × 300 pt page protected with user password "user" and owner password "owner",
/// allowing nothing.
fn protected(algorithm: Algorithm) -> Vec<u8> {
    let mut doc = pdfcraft_cos::Document::open(std::sync::Arc::new(small_page())).unwrap();
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

/// pdfcraft takes the owner password too, also of revision 2–4 files, whose owner password its
/// renderer doesn't take: what reaches the renderer is the user password owner authentication
/// recovers (pdfcraft#2670). A wrong password still shows nothing.
#[test]
fn either_password_opens_a_protected_file_of_any_revision() {
    for algorithm in ALGORITHMS {
        let pdf = tmp("passwords", &format!("{algorithm:?}.pdf"));
        std::fs::write(&pdf, protected(algorithm)).unwrap();
        for password in ["user", "owner"] {
            let out = view(&pdf, &["--password", password, "--protocol", "kitty"]);
            assert!(out.status.success(), "{algorithm:?} with {password:?}: {}", text(&out.stderr));
            let drawn = images(&out.stdout);
            assert_eq!(drawn.len(), 1, "{algorithm:?} with {password:?}");
            let (w, h) = png_size(&drawn[0].png);
            assert!((w as f32 / h as f32 - 200.0 / 300.0).abs() < 0.01, "{algorithm:?} with {password:?}: {w}×{h}");
        }
        let out = view(&pdf, &["--password", "wrong", "--protocol", "kitty"]);
        assert!(!out.status.success() && out.stdout.is_empty(), "{algorithm:?} with a wrong password");
    }
}

/// A dynamic XFA form whose template lays out into two 200 × 300 pt pages, while the file
/// carries one US Letter placeholder page.
fn xfa_form() -> Vec<u8> {
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
    pdf(&[
        "<< /Type /Catalog /Pages 2 0 R /NeedsRendering true /AcroForm << /XFA 5 0 R /Fields [] >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>".into(),
        "<< /Length 0 >>\nstream\n\nendstream".into(),
        format!("<< /Length {} >>\nstream\n{xdp}\nendstream", xdp.len()),
    ])
}

/// A dynamic XFA form shows the pages its template lays out into, as pdfcraft's app does
/// (pdfcraft#1645), not the placeholder page the file carries; the file itself is left alone.
#[test]
fn a_dynamic_xfa_form_shows_its_laid_out_pages() {
    let pdf = tmp("xfa", "form.pdf");
    std::fs::write(&pdf, xfa_form()).unwrap();
    let original = std::fs::read(&pdf).unwrap();
    let out = view(&pdf, &["--protocol", "kitty"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let drawn = images(&out.stdout);
    assert_eq!(drawn.len(), 2, "the form lays out into two pages; the file has one placeholder");
    for image in &drawn {
        let (w, h) = png_size(&image.png);
        // 200 × 300 pt, the form's medium: the placeholder is US Letter.
        assert!((w as f32 / h as f32 - 200.0 / 300.0).abs() < 0.01, "{w}×{h}");
    }
    assert_eq!(std::fs::read(&pdf).unwrap(), original, "viewing must not write the layout to the file");
}

/// A colour pdfcraft can't compute is painted in a fallback colour (pdfcraft#4260): the backend
/// warns, and the viewer says it.
#[test]
fn a_colour_painted_in_a_fallback_colour_is_said() {
    // A Separation colour whose tint transform is of no known function type.
    let content = "/CS1 cs 1 scn 0 0 200 200 re f";
    let bytes = pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] \
         /Resources << /ColorSpace << /CS1 [/Separation /Spot /DeviceRGB << /FunctionType 7 /Domain [0 1] >>] >> >> /Contents 4 0 R >>"
            .into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    ]);
    let pdf = tmp("fallback", "fallback.pdf");
    std::fs::write(&pdf, bytes).unwrap();
    let said = "warning: page 1: a colour that could not be computed was painted in a fallback colour";
    let out = view(&pdf, &["--protocol", "kitty"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(images(&out.stdout).len(), 1, "the page is still drawn");
    assert!(text(&out.stderr).contains(said), "{}", text(&out.stderr));
}

/// Only this backend, its tests, the line that lists it among the backends and the lines that
/// declare its modules name pdfcraft: the rest of the code would read the same with another PDF
/// renderer.
#[test]
fn nothing_but_this_backend_names_pdfcraft() {
    let own = ["src/source/pdfcraft.rs", "tests/it/pdfcraft.rs", "tests/it/pdfcraft_pin.rs"];
    crate::fixtures::named_only_in(&own, &["&pdfcraft::Pdfcraft,", "mod pdfcraft;", "mod pdfcraft_pin;"], &["pdfcraft"]);
}
