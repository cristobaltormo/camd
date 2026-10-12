import { api, b64uToBuf, bufToB64u, deviceLabel, h, icon } from "./lib.js";

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
        id: cred.id,
        rawId: bufToB64u(cred.rawId),
        type: cred.type,
        response: {
          clientDataJSON: bufToB64u(cred.response.clientDataJSON),
          attestationObject: bufToB64u(cred.response.attestationObject),
          transports: cred.response.getTransports ? cred.response.getTransports() : [],
        },
        clientExtensionResults: cred.getClientExtensionResults(),
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
        id: cred.id,
        rawId: bufToB64u(cred.rawId),
        type: cred.type,
        response: {
          clientDataJSON: bufToB64u(cred.response.clientDataJSON),
          authenticatorData: bufToB64u(cred.response.authenticatorData),
          signature: bufToB64u(cred.response.signature),
          userHandle: cred.response.userHandle ? bufToB64u(cred.response.userHandle) : null,
        },
        clientExtensionResults: cred.getClientExtensionResults(),
      },
    },
  });
}

function authError(e) {
  if (e && (e.name === "NotAllowedError" || e.name === "AbortError")) return "Operación cancelada.";
  if (e && e.name === "InvalidStateError") return "Este dispositivo ya tiene una passkey registrada.";
  return (e && e.message) || String(e);
}

export function offlineScreen() {
  return h("div", { class: "auth-wrap" },
    h("div", { class: "auth-head" }, icon("logo", "logo"), h("h1", { text: "Habitación" }), h("p", { class: "muted", text: "No se puede conectar con el servidor." })),
    h("div", { class: "card stack" }, h("button", { class: "btn primary block", text: "Reintentar", onclick: () => location.reload() }))
  );
}

export function authScreen(state, onDone) {
  if (!window.PublicKeyCredential) {
    return h("div", { class: "auth-wrap" }, h("div", { class: "auth-head" }, icon("logo", "logo"), h("h1", { text: "Habitación" }), h("p", { class: "muted", text: "Este navegador no admite passkeys." })));
  }
  const err = h("div", { class: "err", role: "alert" });
  const busy = (btn, label, fn) => async () => {
    err.textContent = "";
    const prev = btn.textContent;
    btn.disabled = true;
    btn.textContent = label;
    try { await fn(); } catch (e) { err.textContent = authError(e); } finally { btn.disabled = false; btn.textContent = prev; }
  };

  const loginBtn = h("button", { class: "btn primary block big", text: "Entrar con passkey" });
  loginBtn.addEventListener("click", busy(loginBtn, "Verificando...", async () => { await loginPasskey(); onDone(); }));

  const codeInput = h("input", { type: "text", placeholder: "Código de alta", autocomplete: "off", autocapitalize: "off", spellcheck: "false", "aria-label": "Código de alta" });
  const enrollBtn = h("button", { class: "btn block " + (state.enrolled ? "" : "primary big"), text: "Registrar este dispositivo" });
  enrollBtn.addEventListener("click", busy(enrollBtn, "Registrando...", async () => {
    const code = codeInput.value.trim();
    if (!code) throw new Error("Introduce el código de alta.");
    await registerPasskey(code);
    onDone();
  }));
  const enrollBox = h("div", { class: "stack", hidden: state.enrolled },
    h("p", { class: "muted", text: "Pide un código de alta desde un dispositivo ya registrado, o genéralo en el servidor." }),
    codeInput, enrollBtn
  );
  const toggle = state.enrolled
    ? h("button", { class: "link", text: "Registrar un dispositivo nuevo", onclick: (e) => { enrollBox.hidden = false; e.currentTarget.hidden = true; codeInput.focus(); } })
    : null;

  return h("div", { class: "auth-wrap" },
    h("div", { class: "auth-head" },
      icon("logo", "logo"),
      h("h1", { text: "Habitación" }),
      h("p", { class: "muted", text: state.enrolled ? "Acceso protegido con passkey" : "Primer acceso: registra este dispositivo" })
    ),
    h("div", { class: "card stack" }, state.enrolled ? loginBtn : null, enrollBox, toggle, err)
  );
}
