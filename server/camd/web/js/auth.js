import { api, b64uToBuf, bufToB64u, deviceLabel, h, icon } from "./lib.js";
import { t } from "./text.js";

const credentialFields = (cred) => ({ id: cred.id, rawId: bufToB64u(cred.rawId), type: cred.type, clientExtensionResults: cred.getClientExtensionResults() });

async function registerPasskey(code) {
  const options = await api("/api/auth/register/options", { method: "POST", body: { code } });
  options.challenge = b64uToBuf(options.challenge);
  options.user.id = b64uToBuf(options.user.id);
  (options.excludeCredentials || []).forEach((c) => (c.id = b64uToBuf(c.id)));
  const cred = await navigator.credentials.create({ publicKey: options });
  await api("/api/auth/register/verify", {
    method: "POST",
    body: {
      code,
      name: deviceLabel(navigator.userAgent).slice(0, 40),
      credential: {
        ...credentialFields(cred),
        response: {
          clientDataJSON: bufToB64u(cred.response.clientDataJSON),
          attestationObject: bufToB64u(cred.response.attestationObject),
          transports: cred.response.getTransports ? cred.response.getTransports() : [],
        },
      },
    },
  });
}

async function loginPasskey() {
  const { cid, options } = await api("/api/auth/login/options", { method: "POST" });
  options.challenge = b64uToBuf(options.challenge);
  (options.allowCredentials || []).forEach((c) => (c.id = b64uToBuf(c.id)));
  const cred = await navigator.credentials.get({ publicKey: options });
  await api("/api/auth/login/verify", {
    method: "POST",
    body: {
      cid,
      credential: {
        ...credentialFields(cred),
        response: {
          clientDataJSON: bufToB64u(cred.response.clientDataJSON),
          authenticatorData: bufToB64u(cred.response.authenticatorData),
          signature: bufToB64u(cred.response.signature),
          userHandle: cred.response.userHandle ? bufToB64u(cred.response.userHandle) : null,
        },
      },
    },
  });
}

function authError(e) {
  if (e && (e.name === "NotAllowedError" || e.name === "AbortError")) return t("Cancelled.");
  if (e && e.name === "InvalidStateError") return t("This device already has a passkey.");
  return (e && e.message) || String(e);
}

const head = (name, text) => h("div", { class: "auth-head" }, icon("logo", "logo"), h("h1", { text: name }), h("p", { class: "muted", text }));

export function offlineScreen(name) {
  return h("div", { class: "auth-wrap" }, head(name, t("Cannot reach the server.")),
    h("div", { class: "card stack" }, h("button", { class: "btn primary block", text: t("Retry"), onclick: () => location.reload() })));
}

export function authScreen(state, onDone) {
  const error = h("div", { class: "err", role: "alert" });
  const guarded = (button, busyLabel, action) => async (event) => {
    event?.preventDefault();
    error.textContent = "";
    const label = button.textContent;
    button.disabled = true;
    button.textContent = busyLabel;
    try { await action(); } catch (e) { error.textContent = authError(e); } finally { button.disabled = false; button.textContent = label; }
  };

  const passkeysUsable = !!window.PublicKeyCredential && window.isSecureContext;
  const sections = [];

  if (state.passkeys && passkeysUsable) {
    const button = h("button", { class: "btn primary block big", text: t("Sign in with passkey") });
    button.addEventListener("click", guarded(button, t("Verifying..."), async () => { await loginPasskey(); onDone(); }));
    sections.push(button);
  }

  if (state.password) {
    const input = h("input", { type: "password", autocomplete: "current-password", placeholder: t("Password"), "aria-label": t("Password") });
    const button = h("button", { type: "submit", class: "btn block " + (sections.length ? "" : "primary big"), text: t("Sign in with password") });
    const form = h("form", { class: "stack" }, input, button);
    form.addEventListener("submit", guarded(button, t("Verifying..."), async () => {
      await api("/api/auth/login/password", { method: "POST", body: { password: input.value } });
      onDone();
    }));
    sections.push(form);
  }

  if (!state.passkeys && passkeysUsable) {
    const code = h("input", { type: "text", placeholder: t("Enrollment code"), autocomplete: "off", autocapitalize: "off", spellcheck: "false", "aria-label": t("Enrollment code") });
    const button = h("button", { class: "btn block", text: t("Register this device") });
    button.addEventListener("click", guarded(button, t("Registering..."), async () => {
      if (!code.value.trim()) throw new Error(t("Enter the enrollment code."));
      await registerPasskey(code.value.trim());
      onDone();
    }));
    sections.push(h("div", { class: "stack" }, h("p", { class: "muted", text: t("Generate a code on the server with camd-ctl enroll.") }), code, button));
  }

  if (!state.passkeys && !state.password) {
    sections.push(h("p", { class: "muted", text: t("No way to sign in is set up yet. On the server run camd-ctl password, or camd-ctl enroll for a passkey.") }));
  }
  if (state.passkeys && !passkeysUsable && !state.password) {
    sections.push(h("p", { class: "muted", text: t("Passkeys need HTTPS. Open the camera through its secure address, or set a password with camd-ctl password.") }));
  }

  const subtitle = state.passkeys || state.password ? t("Sign in to continue") : t("First start");
  return h("div", { class: "auth-wrap" }, head(state.site_name, subtitle), h("div", { class: "card stack" }, ...sections, error));
}
