import { useEffect, useState } from "react";
import { useApi } from "../api";
import { formatDate } from "../signatures";
import { toPwError, type ChainCertificate, type OpenedDoc, type PwError, type TrustedCert } from "../types";
import { Dialog } from "./ui";

/** Choose which certificate of a signature's chain to trust: the signer only, or an issuer. */
export function TrustSignerDialog({ doc, index, onClose, onTrusted }: { doc: OpenedDoc; index: number; onClose: () => void; onTrusted: (c: TrustedCert) => void }) {
  const api = useApi();
  const [chain, setChain] = useState<ChainCertificate[] | null>(null);
  const [choice, setChoice] = useState(0);
  const [error, setError] = useState<PwError | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let live = true;
    api
      .signatureChain(doc.doc_id, index)
      .then((c) => live && setChain(c))
      .catch((e) => live && setError(toPwError(e)));
    return () => {
      live = false;
    };
  }, [api, doc.doc_id, index]);

  const chosen = chain?.[choice];
  const trust = async () => {
    if (!chosen || busy) return;
    setBusy(true);
    try {
      onTrusted(await api.trustCertificate(doc.doc_id, index, choice));
      onClose();
    } catch (e) {
      setError(toPwError(e));
      setBusy(false);
    }
  };

  const footer = (
    <>
      <button type="button" className="btn outline" onClick={onClose} disabled={busy}>
        Cancel
      </button>
      <button type="button" className="btn primary" data-testid="trust-confirm" aria-disabled={!chosen || busy ? true : undefined} title={chosen ? undefined : "Loading the certificates…"} onClick={trust}>
        Trust
      </button>
    </>
  );

  return (
    <Dialog title="Trust this signer" onClose={onClose} footer={footer} busy={busy}>
      <p className="hint" style={{ marginTop: 0 }}>
        Only trust a certificate after confirming its fingerprint with its owner (for example by phone). Signatures that chain to it will then show as trusted.
      </p>
      {chain === null && !error && <p className="hint">Reading the signature’s certificates…</p>}
      {chain && (
        <fieldset className="field" style={{ border: 0, padding: 0, margin: 0, minWidth: 0 }}>
          <legend className="label">Trust</legend>
          {chain.map((c, i) => (
            <label key={c.fingerprint} className="trust-option">
              <input type="radio" name="trust-choice" checked={choice === i} data-trust-position={i} onChange={() => setChoice(i)} />
              <span className="grow">
                <span className="name">{i === 0 ? `Only ${c.name}` : `Everything issued by ${c.name}`}</span>
                <span className="hint">
                  {i === 0 ? "This signer’s certificate." : c.self_signed ? "A root certificate authority." : "An intermediate certificate authority."} Valid to {formatDate(c.valid_to)}.
                </span>
                <code className="fingerprint" aria-label={`SHA-256 fingerprint of ${c.name}`}>
                  {c.fingerprint}
                </code>
              </span>
            </label>
          ))}
        </fieldset>
      )}
      {error && (
        <div className="banner error" role="alert" style={{ borderRadius: 6 }}>
          {error.message}
        </div>
      )}
    </Dialog>
  );
}

/** The certificates the user trusts, with Remove. Changes apply immediately. */
export function TrustedCertificatesDialog({ onClose, onChanged }: { onClose: () => void; onChanged: () => void }) {
  const api = useApi();
  const [list, setList] = useState<TrustedCert[] | null>(null);
  const [error, setError] = useState<PwError | null>(null);

  const load = () =>
    api
      .trustedCertificates()
      .then(setList)
      .catch((e) => setError(toPwError(e)));
  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const remove = async (c: TrustedCert) => {
    try {
      await api.removeTrustedCertificate(c.fingerprint);
      onChanged();
      await load();
    } catch (e) {
      setError(toPwError(e));
    }
  };

  const footer = (
    <button type="button" className="btn primary" onClick={onClose}>
      Done
    </button>
  );

  return (
    <Dialog title="Trusted certificates" onClose={onClose} footer={footer}>
      <p className="hint" style={{ marginTop: 0 }}>
        Signatures whose certificates chain to one of these show as trusted. Revocation is not checked. Changes apply immediately.
      </p>
      {list === null && !error && <p className="hint">Loading…</p>}
      {list?.length === 0 && (
        <p className="hint" data-testid="trusted-none">
          You do not trust any certificates yet. Use “Trust…” on a signature in the Signatures panel.
        </p>
      )}
      {list && list.length > 0 && (
        <ul className="trusted-list" aria-label="Trusted certificates">
          {list.map((c) => (
            <li key={c.fingerprint} className="row" data-trusted={c.name}>
              <span className="grow">
                <span className="name">{c.name}</span>
                <span className="hint"> · valid to {formatDate(c.valid_to)}</span>
                <code className="fingerprint">{c.fingerprint}</code>
              </span>
              <button type="button" className="btn outline" aria-label={`Remove ${c.name} from trusted certificates`} onClick={() => remove(c)}>
                Remove
              </button>
            </li>
          ))}
        </ul>
      )}
      {error && (
        <div className="banner error" role="alert" style={{ borderRadius: 6 }}>
          {error.message}
        </div>
      )}
    </Dialog>
  );
}
