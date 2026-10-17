//! Single dispatcher from the typed [`OperationRequest`] to the engine — the one implementation that
//! the desktop UI, CLI, local API and MCP all call (PRD 04 "Shared engine").

use crate::document::{open_document, page_count, page_texts, render_page_png, search, OpenOptions};
use crate::edit;
use pw_core::job::JobContext;
use pw_core::ops::{Operation, OperationRequest, OperationResult, OutputInfo, SCHEMA_VERSION};
use pw_core::output::write_atomic_bytes;
use pw_core::range::parse_page_ranges;
use pw_core::PwResult;
use serde_json::json;
use std::time::Instant;

fn out(c: &pw_core::output::CommittedOutput, pages: Option<u32>) -> OutputInfo {
    OutputInfo { path: c.path.clone(), bytes: c.bytes, pages }
}

/// Execute a request. `ctx` provides cancellation and progress when run as a job.
pub fn execute(req: &OperationRequest, ctx: Option<&JobContext>) -> PwResult<OperationResult> {
    req.validate()?;
    let started = Instant::now();
    let cancelled = || ctx.map(|c| c.is_cancelled()).unwrap_or(false);
    let progress = |done: u64, total: u64| {
        if let Some(c) = ctx {
            c.set_progress(done, total, "files");
        }
    };
    if let Some(c) = ctx {
        c.set_phase(req.operation.id());
    }
    let opts = OpenOptions { password: req.password.as_deref() };
    let policy = req.collision;
    let mut warnings = Vec::new();
    let (outputs, data) = match &req.operation {
        Operation::Inspect { input } => {
            let info = open_document(input, &opts)?;
            warnings.extend(info.warnings.iter().cloned());
            (vec![], serde_json::to_value(&info).unwrap_or_default())
        }
        Operation::FormFields { input } => {
            let info = crate::forms::form_fields(input, opts.password)?;
            if info.has_xfa {
                warnings.push("This form uses XFA; its fields cannot be filled here.".into());
            }
            (vec![], serde_json::to_value(&info).unwrap_or_default())
        }
        Operation::ExtractText { input, pages } => {
            let n = page_count(input, &opts)?;
            let idx = match pages {
                Some(spec) => parse_page_ranges(spec, n)?,
                None => (0..n).collect(),
            };
            let texts = page_texts(input, &opts, &idx)?;
            let pages: Vec<_> = texts.into_iter().map(|(p, t)| json!({"page": p + 1, "text": t})).collect();
            (vec![], json!({ "pages": pages }))
        }
        Operation::Search { input, query, match_case, whole_word } => {
            let (hits, truncated) = search(input, &opts, query, *match_case, *whole_word, &cancelled)?;
            if truncated {
                warnings.push(format!("Showing the first {} matches.", hits.len()));
            }
            (vec![], json!({ "count": hits.len(), "truncated": truncated, "hits": hits }))
        }
        Operation::Render { input, page, scale, output } => {
            let (png, w, h) = render_page_png(input, &opts, page - 1, *scale)?;
            let c = write_atomic_bytes(output, policy, &req.operation.inputs(), &png, |p| {
                let back = std::fs::read(p).map_err(|e| pw_core::PwError::from_io(&e, "output file"))?;
                if back.starts_with(b"\x89PNG\r\n\x1a\n") {
                    Ok(())
                } else {
                    Err(pw_core::PwError::new(
                        pw_core::ErrorCategory::ValidationFailed,
                        "output.verify",
                        "The rendered image failed verification.",
                    ))
                }
            })?;
            (vec![out(&c, None)], json!({ "width": w, "height": h }))
        }
        Operation::Merge { inputs, output } => {
            let (c, n, w) = edit::merge(inputs, &opts, output, policy, &cancelled, &progress)?;
            warnings.extend(w);
            (vec![out(&c, Some(n))], json!({ "page_count": n }))
        }
        Operation::ExtractPages { input, pages, output } => {
            let (c, n, w) = edit::extract_pages(input, &opts, pages, output, policy)?;
            warnings.extend(w);
            (vec![out(&c, Some(n))], json!({ "page_count": n }))
        }
        Operation::DeletePages { input, pages, output } => {
            let (c, n, w) = edit::delete_pages(input, &opts, pages, output, policy)?;
            warnings.extend(w);
            (vec![out(&c, Some(n))], json!({ "page_count": n }))
        }
        Operation::RotatePages { input, pages, degrees, output } => {
            let (c, n, w) = edit::rotate_pages(input, &opts, pages.as_deref(), *degrees, output, policy)?;
            warnings.extend(w);
            (vec![out(&c, Some(n))], json!({ "page_count": n }))
        }
        Operation::Annotate { input, annotations, output } => {
            let (c, n, w) = edit::annotate(input, &opts, annotations, output, policy)?;
            warnings.extend(w);
            (vec![out(&c, Some(n))], json!({ "page_count": n, "annotations": annotations.len() }))
        }
        Operation::FillForm { input, values, output } => {
            let (c, w) = crate::forms::fill_form(input, opts.password, values, output, policy)?;
            warnings.extend(w);
            (vec![out(&c, None)], json!({ "fields": values.len() }))
        }
        Operation::OrganizePages { input, extra_inputs, pages, output } => {
            let (c, n, w) = edit::organize_pages(input, &opts, extra_inputs, pages, output, policy, &cancelled)?;
            warnings.extend(w);
            (vec![out(&c, Some(n))], json!({ "page_count": n }))
        }
        Operation::ExportImages { input, pages, dpi, output_dir } => {
            let (files, w) =
                edit::export_images(input, &opts, pages.as_deref(), *dpi, output_dir, policy, &cancelled, &progress)?;
            warnings.extend(w);
            let n = files.len();
            (files.iter().map(|c| out(c, None)).collect(), json!({ "images": n }))
        }
        Operation::StampText { input, pages, stamps, start, digits, output } => {
            let (c, n, w) = edit::stamp_text(input, &opts, pages.as_deref(), stamps, *start, *digits, output, policy)?;
            warnings.extend(w);
            (vec![out(&c, Some(n))], json!({ "page_count": n }))
        }
        Operation::Protect { input, permissions, output } => {
            let user = req.new_password.as_deref().unwrap_or_default();
            let (c, n, w) = crate::protect::protect(
                input,
                &opts,
                user,
                req.new_owner_password.as_deref(),
                permissions,
                output,
                policy,
            )?;
            warnings.extend(w);
            (vec![out(&c, Some(n))], json!({ "page_count": n, "encryption": "AES-256" }))
        }
        Operation::OcrPages { input, pages, skip_text_pages, dpi, output } => {
            let page_progress = |done: u64, total: u64| {
                if let Some(c) = ctx {
                    c.set_progress(done, total, "pages");
                }
            };
            let (c, n, summary, w) = crate::ocr::ocr_pages(
                input,
                &opts,
                pages.as_deref(),
                *skip_text_pages,
                *dpi,
                output,
                policy,
                &cancelled,
                &page_progress,
            )?;
            warnings.extend(w);
            (vec![out(&c, Some(n))], serde_json::to_value(&summary).unwrap_or_default())
        }
        Operation::SanitizeInspect { input } => {
            let found = crate::sanitize::sanitize_inspect(input, &opts)?;
            (vec![], serde_json::to_value(&found).unwrap_or_default())
        }
        Operation::Sanitize { input, options, output } => {
            let page_progress = |done: u64, total: u64| {
                if let Some(c) = ctx {
                    c.set_progress(done, total, "pages");
                }
            };
            let (c, n, removed, w) =
                crate::sanitize::sanitize(input, &opts, options, output, policy, &cancelled, &page_progress)?;
            warnings.extend(w);
            (vec![out(&c, Some(n))], json!({ "page_count": n, "removed": removed }))
        }
        Operation::RedactionPreview { input, areas } => {
            let previews = crate::redact::redaction_preview(input, &opts, areas)?;
            (vec![], json!({ "areas": previews }))
        }
        Operation::Redact { input, areas, output } => {
            let (c, n, w) = crate::redact::redact(input, &opts, areas, output, policy, &cancelled)?;
            warnings.extend(w);
            (vec![out(&c, Some(n))], json!({ "page_count": n, "areas": areas.len() }))
        }
        Operation::ApplyEdits { input, form_values, annotations, output } => {
            let (c, w) = crate::forms::apply_edits(input, opts.password, form_values, annotations, output, policy)?;
            warnings.extend(w);
            (vec![out(&c, None)], json!({ "fields": form_values.len(), "annotations": annotations.len() }))
        }
        Operation::Split { input, split, output_dir } => {
            let (parts, w) = edit::split(input, &opts, split, output_dir, policy, &cancelled)?;
            warnings.extend(w);
            let outs: Vec<OutputInfo> = parts.iter().map(|(c, n)| out(c, Some(*n))).collect();
            let n = outs.len();
            (outs, json!({ "parts": n }))
        }
    };
    Ok(OperationResult {
        schema_version: SCHEMA_VERSION,
        operation: req.operation.id().to_string(),
        outputs,
        data,
        warnings,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}
