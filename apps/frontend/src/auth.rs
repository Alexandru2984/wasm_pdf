use gloo::net::http::{Request, Response};
use gloo::timers::callback::Interval;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;
use uuid::Uuid;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{HtmlInputElement, InputEvent, RequestCredentials, SubmitEvent};
use yew::prelude::*;

const CSRF_STORAGE_KEY: &str = "pdf_editor_csrf";
const SESSION_REFRESH_INTERVAL_MS: u32 = 10 * 60 * 1_000;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = pdfEditorWebAuthn, js_name = createCredential, catch)]
    async fn create_credential(options: JsValue) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = pdfEditorWebAuthn, js_name = getCredential, catch)]
    async fn get_credential(options: JsValue) -> Result<JsValue, JsValue>;
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct PublicUser {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub mfa_required: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AuthSession {
    pub access_token: String,
    pub csrf_token: String,
    pub user: PublicUser,
}

#[derive(Debug, Deserialize)]
struct AuthResponse {
    access_token: String,
    csrf_token: String,
    user: PublicUser,
}

impl From<AuthResponse> for AuthSession {
    fn from(value: AuthResponse) -> Self {
        Self {
            access_token: value.access_token,
            csrf_token: value.csrf_token,
            user: value.user,
        }
    }
}

#[derive(Debug, Deserialize)]
struct MfaChallenge {
    ceremony_id: Uuid,
    public_key: Value,
}

#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    message: String,
}

#[derive(Properties, PartialEq)]
pub struct AuthPanelProps {
    pub session: Option<AuthSession>,
    pub on_session: Callback<Option<AuthSession>>,
}

#[function_component(AuthPanel)]
pub fn auth_panel(props: &AuthPanelProps) -> Html {
    let register_mode = use_state(|| false);
    let display_name = use_state(String::new);
    let email = use_state(String::new);
    let password = use_state(String::new);
    let backup_code = use_state(String::new);
    let mfa_challenge = use_state(|| None::<MfaChallenge>);
    let busy = use_state(|| false);
    let error = use_state(|| None::<String>);
    let settings_open = use_state(|| false);

    {
        let on_session = props.on_session.clone();
        use_effect_with((), move |()| {
            if let Some(csrf_token) = load_csrf() {
                spawn_local(async move {
                    match refresh_session(&csrf_token).await {
                        Ok(session) => set_authenticated(&on_session, session),
                        Err(_) => clear_auth(&on_session),
                    }
                });
            }
            || ()
        });
    }

    {
        let csrf_token = props
            .session
            .as_ref()
            .map(|session| session.csrf_token.clone());
        let on_session = props.on_session.clone();
        use_effect_with(csrf_token, move |csrf_token| {
            let interval = csrf_token.clone().map(|csrf_token| {
                Interval::new(SESSION_REFRESH_INTERVAL_MS, move || {
                    let csrf_token = csrf_token.clone();
                    let on_session = on_session.clone();
                    spawn_local(async move {
                        if let Ok(session) = refresh_session(&csrf_token).await {
                            set_authenticated(&on_session, session);
                        }
                    });
                })
            });
            move || drop(interval)
        });
    }

    let on_submit = {
        let register_mode = register_mode.clone();
        let display_name = display_name.clone();
        let email = email.clone();
        let password = password.clone();
        let mfa_challenge = mfa_challenge.clone();
        let busy = busy.clone();
        let error = error.clone();
        let on_session = props.on_session.clone();
        Callback::from(move |event: SubmitEvent| {
            event.prevent_default();
            if *busy {
                return;
            }
            busy.set(true);
            error.set(None);
            mfa_challenge.set(None);
            let register = *register_mode;
            let body = if register {
                serde_json::json!({
                    "email": email.trim(),
                    "display_name": display_name.trim(),
                    "password": password.as_str(),
                })
            } else {
                serde_json::json!({
                    "email": email.trim(),
                    "password": password.as_str(),
                })
            };
            let busy = busy.clone();
            let error = error.clone();
            let mfa_challenge = mfa_challenge.clone();
            let password = password.clone();
            let on_session = on_session.clone();
            spawn_local(async move {
                let result = if register {
                    authenticate("/api/v1/auth/register", &body).await
                } else {
                    login(&body).await
                };
                match result {
                    Ok(LoginResult::Authenticated(session)) => {
                        password.set(String::new());
                        set_authenticated(&on_session, session);
                    }
                    Ok(LoginResult::MfaRequired(challenge)) => {
                        password.set(String::new());
                        match finish_browser_mfa(&challenge).await {
                            Ok(session) => set_authenticated(&on_session, session),
                            Err(message) => {
                                error.set(Some(message));
                                mfa_challenge.set(Some(challenge));
                            }
                        }
                    }
                    Err(message) => error.set(Some(message)),
                }
                busy.set(false);
            });
        })
    };

    let on_backup_code = {
        let backup_code = backup_code.clone();
        let mfa_challenge = mfa_challenge.clone();
        let busy = busy.clone();
        let error = error.clone();
        let on_session = props.on_session.clone();
        Callback::from(move |_| {
            let Some(challenge) = mfa_challenge.as_ref() else {
                return;
            };
            if *busy {
                return;
            }
            busy.set(true);
            error.set(None);
            let ceremony_id = challenge.ceremony_id;
            let code = (*backup_code).clone();
            let busy = busy.clone();
            let error = error.clone();
            let backup_code = backup_code.clone();
            let on_session = on_session.clone();
            spawn_local(async move {
                let body = serde_json::json!({ "ceremony_id": ceremony_id, "code": code });
                match authenticate("/api/v1/auth/mfa/backup-code", &body).await {
                    Ok(LoginResult::Authenticated(session)) => {
                        backup_code.set(String::new());
                        set_authenticated(&on_session, session);
                    }
                    Ok(LoginResult::MfaRequired(_)) => {
                        error.set(Some("Răspuns MFA neașteptat.".to_owned()));
                    }
                    Err(message) => error.set(Some(message)),
                }
                busy.set(false);
            });
        })
    };

    let on_logout = {
        let session = props.session.clone();
        let on_session = props.on_session.clone();
        let busy = busy.clone();
        let error = error.clone();
        Callback::from(move |_| {
            let Some(session) = session.clone() else {
                return;
            };
            busy.set(true);
            let busy = busy.clone();
            let error = error.clone();
            let on_session = on_session.clone();
            spawn_local(async move {
                if let Err(message) = logout(&session.csrf_token).await {
                    error.set(Some(message));
                }
                clear_auth(&on_session);
                busy.set(false);
            });
        })
    };

    html! {
        <section class="account-panel" aria-labelledby="account-title">
            if let Some(session) = &props.session {
                <div class="account-summary">
                    <div>
                        <p class="step">{"CONT ACTIV"}</p>
                        <h2 id="account-title">{&session.user.display_name}</h2>
                        <p class="account-email">{&session.user.email}</p>
                    </div>
                    <div class="account-actions">
                        <button class="secondary-button" type="button" onclick={{
                            let settings_open = settings_open.clone();
                            Callback::from(move |_| settings_open.set(!*settings_open))
                        }}>
                            {if *settings_open { "Închide setările" } else { "Securitate cont" }}
                        </button>
                        <button class="secondary-button" type="button" onclick={on_logout} disabled={*busy}>
                            {"Logout"}
                        </button>
                    </div>
                </div>
                if *settings_open {
                    <AccountSecurity session={session.clone()} on_session={props.on_session.clone()} />
                }
            } else {
                <div class="section-heading compact-heading">
                    <div>
                        <p class="step">{"CONT OPȚIONAL"}</p>
                        <h2 id="account-title">{if *register_mode { "Creează cont" } else { "Autentificare" }}</h2>
                    </div>
                    <button class="text-button" type="button" onclick={{
                        let register_mode = register_mode.clone();
                        let error = error.clone();
                        Callback::from(move |_| {
                            register_mode.set(!*register_mode);
                            error.set(None);
                        })
                    }}>
                        {if *register_mode { "Am deja cont" } else { "Cont nou" }}
                    </button>
                </div>
                <form class="auth-form" onsubmit={on_submit}>
                    if *register_mode {
                        <AuthInput id="display-name" label="Nume afișat" value={display_name.clone()} input_type="text" autocomplete="name" />
                    }
                    <AuthInput id="auth-email" label="Email" value={email.clone()} input_type="email" autocomplete="email" />
                    <AuthInput id="auth-password" label="Parolă" value={password.clone()} input_type="password" autocomplete={if *register_mode { "new-password" } else { "current-password" }} />
                    <button class="process-button account-submit" type="submit" disabled={*busy}>
                        {if *busy { "Se verifică…" } else if *register_mode { "Creează cont" } else { "Intră în cont" }}
                    </button>
                </form>
                if let Some(challenge) = mfa_challenge.as_ref() {
                    <div class="mfa-fallback">
                        <p>{format!("Passkey-ul nu a putut fi folosit. Ceremony: {}", challenge.ceremony_id)}</p>
                        <AuthInput id="backup-code" label="Cod de backup" value={backup_code.clone()} input_type="text" autocomplete="one-time-code" />
                        <button class="secondary-button" type="button" onclick={on_backup_code} disabled={*busy}>{"Folosește codul"}</button>
                    </div>
                }
            }
            if let Some(message) = &*error {
                <div class="notice error" role="alert">{message}</div>
            }
        </section>
    }
}

#[derive(Properties, PartialEq)]
struct AuthInputProps {
    id: &'static str,
    label: &'static str,
    value: UseStateHandle<String>,
    input_type: &'static str,
    autocomplete: &'static str,
}

#[function_component(AuthInput)]
fn auth_input(props: &AuthInputProps) -> Html {
    let value = props.value.clone();
    html! {
        <div class="auth-field">
            <label class="field-label" for={props.id}>{props.label}</label>
            <input
                id={props.id}
                type={props.input_type}
                value={(*props.value).clone()}
                autocomplete={props.autocomplete}
                oninput={Callback::from(move |event: InputEvent| {
                    value.set(event.target_unchecked_into::<HtmlInputElement>().value());
                })}
                required=true
            />
        </div>
    }
}

#[derive(Properties, PartialEq)]
struct AccountSecurityProps {
    session: AuthSession,
    on_session: Callback<Option<AuthSession>>,
}

#[function_component(AccountSecurity)]
fn account_security(props: &AccountSecurityProps) -> Html {
    let sessions = use_state(Vec::<SessionSummary>::new);
    let passkeys = use_state(Vec::<PasskeySummary>::new);
    let backup_count = use_state(|| 0_i64);
    let current_password = use_state(String::new);
    let change_current_password = use_state(String::new);
    let new_password = use_state(String::new);
    let display_name = use_state(|| props.session.user.display_name.clone());
    let nickname = use_state(|| "Dispozitivul meu".to_owned());
    let backup_codes = use_state(Vec::<String>::new);
    let busy = use_state(|| false);
    let message = use_state(|| None::<String>);

    {
        let token = props.session.access_token.clone();
        let sessions = sessions.clone();
        let passkeys = passkeys.clone();
        let backup_count = backup_count.clone();
        let message = message.clone();
        use_effect_with(token.clone(), move |_| {
            spawn_local(async move {
                if let Err(error) =
                    reload_security(&token, &sessions, &passkeys, &backup_count).await
                {
                    message.set(Some(error));
                }
            });
            || ()
        });
    }

    let change_password = {
        let session = props.session.clone();
        let current_password = change_current_password.clone();
        let new_password = new_password.clone();
        let busy = busy.clone();
        let message = message.clone();
        let on_session = props.on_session.clone();
        Callback::from(move |event: SubmitEvent| {
            event.prevent_default();
            busy.set(true);
            message.set(None);
            let body = serde_json::json!({
                "current_password": current_password.as_str(),
                "new_password": new_password.as_str(),
            });
            let token = session.access_token.clone();
            let busy = busy.clone();
            let message = message.clone();
            let current_password = current_password.clone();
            let new_password = new_password.clone();
            let on_session = on_session.clone();
            spawn_local(async move {
                match authorized_json::<AuthResponse>("PUT", "/api/v1/auth/password", &token, &body)
                    .await
                {
                    Ok(response) => {
                        current_password.set(String::new());
                        new_password.set(String::new());
                        set_authenticated(&on_session, response.into());
                        message.set(Some("Parola și sesiunile au fost rotate.".to_owned()));
                    }
                    Err(error) => message.set(Some(error)),
                }
                busy.set(false);
            });
        })
    };

    let register_passkey = {
        let session = props.session.clone();
        let nickname = nickname.clone();
        let current_password = current_password.clone();
        let sessions = sessions.clone();
        let passkeys = passkeys.clone();
        let backup_count = backup_count.clone();
        let backup_codes = backup_codes.clone();
        let busy = busy.clone();
        let message = message.clone();
        Callback::from(move |_| {
            busy.set(true);
            message.set(None);
            let token = session.access_token.clone();
            let body = serde_json::json!({
                "nickname": nickname.as_str(),
                "password": current_password.as_str(),
            });
            let sessions = sessions.clone();
            let passkeys = passkeys.clone();
            let backup_count = backup_count.clone();
            let backup_codes = backup_codes.clone();
            let busy = busy.clone();
            let message = message.clone();
            let current_password = current_password.clone();
            spawn_local(async move {
                match enroll_passkey(&token, &body).await {
                    Ok(codes) => {
                        current_password.set(String::new());
                        backup_codes.set(codes);
                        if let Err(error) =
                            reload_security(&token, &sessions, &passkeys, &backup_count).await
                        {
                            message.set(Some(error));
                        } else {
                            message.set(Some("Passkey înregistrat.".to_owned()));
                        }
                    }
                    Err(error) => message.set(Some(error)),
                }
                busy.set(false);
            });
        })
    };

    let regenerate_codes = {
        let token = props.session.access_token.clone();
        let current_password = current_password.clone();
        let backup_codes = backup_codes.clone();
        let backup_count = backup_count.clone();
        let busy = busy.clone();
        let message = message.clone();
        Callback::from(move |_| {
            busy.set(true);
            message.set(None);
            let body = serde_json::json!({ "password": current_password.as_str() });
            let token = token.clone();
            let backup_codes = backup_codes.clone();
            let backup_count = backup_count.clone();
            let busy = busy.clone();
            let message = message.clone();
            let current_password = current_password.clone();
            spawn_local(async move {
                match authorized_json::<BackupCodesResponse>(
                    "POST",
                    "/api/v1/auth/mfa/backup-codes/regenerate",
                    &token,
                    &body,
                )
                .await
                {
                    Ok(response) => {
                        current_password.set(String::new());
                        let count = i64::try_from(response.backup_codes.len()).unwrap_or(i64::MAX);
                        backup_count.set(count);
                        backup_codes.set(response.backup_codes);
                        message.set(Some("Codurile anterioare au fost invalidate.".to_owned()));
                    }
                    Err(error) => message.set(Some(error)),
                }
                busy.set(false);
            });
        })
    };

    let disable_mfa = {
        let session = props.session.clone();
        let current_password = current_password.clone();
        let sessions = sessions.clone();
        let passkeys = passkeys.clone();
        let backup_count = backup_count.clone();
        let busy = busy.clone();
        let message = message.clone();
        let on_session = props.on_session.clone();
        Callback::from(move |_| {
            busy.set(true);
            message.set(None);
            let body = serde_json::json!({ "password": current_password.as_str() });
            let session = session.clone();
            let sessions = sessions.clone();
            let passkeys = passkeys.clone();
            let backup_count = backup_count.clone();
            let busy = busy.clone();
            let message = message.clone();
            let current_password = current_password.clone();
            let on_session = on_session.clone();
            spawn_local(async move {
                match authorized_json_empty(
                    "POST",
                    "/api/v1/auth/mfa/disable",
                    &session.access_token,
                    &body,
                )
                .await
                {
                    Ok(()) => {
                        current_password.set(String::new());
                        let mut updated = session;
                        updated.user.mfa_required = false;
                        let token = updated.access_token.clone();
                        set_authenticated(&on_session, updated);
                        let _ = reload_security(&token, &sessions, &passkeys, &backup_count).await;
                        message.set(Some("MFA a fost dezactivat.".to_owned()));
                    }
                    Err(error) => message.set(Some(error)),
                }
                busy.set(false);
            });
        })
    };

    let update_profile = {
        let session = props.session.clone();
        let display_name = display_name.clone();
        let busy = busy.clone();
        let message = message.clone();
        let on_session = props.on_session.clone();
        Callback::from(move |event: SubmitEvent| {
            event.prevent_default();
            busy.set(true);
            message.set(None);
            let body = serde_json::json!({ "display_name": display_name.trim() });
            let session = session.clone();
            let busy = busy.clone();
            let message = message.clone();
            let on_session = on_session.clone();
            spawn_local(async move {
                match authorized_json::<MeResponse>(
                    "PUT",
                    "/api/v1/auth/profile",
                    &session.access_token,
                    &body,
                )
                .await
                {
                    Ok(response) => {
                        let updated = AuthSession {
                            user: response.user,
                            ..session
                        };
                        set_authenticated(&on_session, updated);
                        message.set(Some("Profil actualizat.".to_owned()));
                    }
                    Err(error) => message.set(Some(error)),
                }
                busy.set(false);
            });
        })
    };

    let delete_account = {
        let token = props.session.access_token.clone();
        let current_password = current_password.clone();
        let busy = busy.clone();
        let message = message.clone();
        let on_session = props.on_session.clone();
        Callback::from(move |_| {
            let confirmed = web_sys::window()
                .and_then(|window| {
                    window
                        .confirm_with_message(
                            "Ștergi definitiv contul și toate datele de identitate?",
                        )
                        .ok()
                })
                .unwrap_or(false);
            if !confirmed {
                return;
            }
            busy.set(true);
            message.set(None);
            let body = serde_json::json!({ "password": current_password.as_str() });
            let token = token.clone();
            let busy = busy.clone();
            let message = message.clone();
            let on_session = on_session.clone();
            spawn_local(async move {
                match authorized_json_empty("DELETE", "/api/v1/auth/account", &token, &body).await {
                    Ok(()) => clear_auth(&on_session),
                    Err(error) => message.set(Some(error)),
                }
                busy.set(false);
            });
        })
    };

    let revoke_others = action_without_body(
        props.session.access_token.clone(),
        "/api/v1/auth/sessions/others/revoke",
        "POST",
        "Celelalte sesiuni au fost revocate.",
        busy.clone(),
        message.clone(),
        Some((sessions.clone(), passkeys.clone(), backup_count.clone())),
    );

    html! {
        <div class="security-grid">
            <div class="security-card">
                <h3>{"Sesiuni active"}</h3>
                <div class="device-list">
                    {sessions.iter().map(|item| {
                        let token = props.session.access_token.clone();
                        let id = item.id;
                        let sessions = sessions.clone();
                        let passkeys = passkeys.clone();
                        let backup_count = backup_count.clone();
                        let busy = busy.clone();
                        let message = message.clone();
                        html! {
                            <div class="device-row">
                                <div>
                                    <strong>{if item.current { "Sesiunea curentă" } else { "Alt dispozitiv" }}</strong>
                                    <small>{format!("{} · {}", item.ip_address.as_deref().unwrap_or("IP necunoscut"), item.user_agent.as_deref().unwrap_or("Browser necunoscut"))}</small>
                                    <small>{format!("Expiră {}", item.expires_at)}</small>
                                </div>
                                if !item.current {
                                    <button class="text-button danger" type="button" onclick={Callback::from(move |_| {
                                        let token = token.clone();
                                        let sessions = sessions.clone();
                                        let passkeys = passkeys.clone();
                                        let backup_count = backup_count.clone();
                                        let busy = busy.clone();
                                        let message = message.clone();
                                        spawn_local(async move {
                                            busy.set(true);
                                            let path = format!("/api/v1/auth/sessions/{id}");
                                            match authorized_empty("DELETE", &path, &token).await {
                                                Ok(()) => {
                                                    let _ = reload_security(&token, &sessions, &passkeys, &backup_count).await;
                                                    message.set(Some("Sesiune revocată.".to_owned()));
                                                }
                                                Err(error) => message.set(Some(error)),
                                            }
                                            busy.set(false);
                                        });
                                    })}>{"Revocă"}</button>
                                }
                            </div>
                        }
                    }).collect::<Html>()}
                </div>
                <button class="secondary-button" type="button" onclick={revoke_others} disabled={*busy}>{"Revocă toate celelalte"}</button>
            </div>

            <div class="security-card">
                <h3>{"Passkeys și MFA"}</h3>
                <p>{format!("{} passkey(s) · {} coduri de backup rămase", passkeys.len(), *backup_count)}</p>
                <div class="device-list">
                    {passkeys.iter().map(|passkey| {
                        let id = passkey.id;
                        let token = props.session.access_token.clone();
                        let current_password = current_password.clone();
                        let sessions = sessions.clone();
                        let passkeys = passkeys.clone();
                        let backup_count = backup_count.clone();
                        let busy = busy.clone();
                        let message = message.clone();
                        html! {
                            <div class="device-row">
                                <strong>{&passkey.nickname}</strong>
                                <button class="text-button danger" type="button" onclick={Callback::from(move |_| {
                                    let path = format!("/api/v1/auth/passkeys/{id}");
                                    let body = serde_json::json!({ "password": current_password.as_str() });
                                    let token = token.clone();
                                    let sessions = sessions.clone();
                                    let passkeys = passkeys.clone();
                                    let backup_count = backup_count.clone();
                                    let busy = busy.clone();
                                    let message = message.clone();
                                    spawn_local(async move {
                                        busy.set(true);
                                        match authorized_json_empty("DELETE", &path, &token, &body).await {
                                            Ok(()) => {
                                                let _ = reload_security(&token, &sessions, &passkeys, &backup_count).await;
                                                message.set(Some("Passkey eliminat.".to_owned()));
                                            }
                                            Err(error) => message.set(Some(error)),
                                        }
                                        busy.set(false);
                                    });
                                })}>{"Elimină"}</button>
                            </div>
                        }
                    }).collect::<Html>()}
                </div>
                <AuthInput id="passkey-nickname" label="Nume passkey" value={nickname.clone()} input_type="text" autocomplete="off" />
                <AuthInput id="security-password" label="Parola curentă" value={current_password.clone()} input_type="password" autocomplete="current-password" />
                <button class="secondary-button" type="button" onclick={register_passkey} disabled={*busy}>{"Adaugă passkey"}</button>
                if !passkeys.is_empty() {
                    <button class="secondary-button" type="button" onclick={regenerate_codes} disabled={*busy}>{"Generează alte coduri"}</button>
                    <button class="secondary-button danger-button" type="button" onclick={disable_mfa} disabled={*busy}>{"Dezactivează MFA"}</button>
                }
                if !backup_codes.is_empty() {
                    <div class="backup-codes" role="status">
                        <strong>{"Salvează acum codurile; nu vor mai fi afișate."}</strong>
                        {backup_codes.iter().map(|code| html! { <code>{code}</code> }).collect::<Html>()}
                    </div>
                }
            </div>

            <form class="security-card" onsubmit={change_password}>
                <h3>{"Schimbă parola"}</h3>
                <AuthInput id="change-current-password" label="Parola curentă" value={change_current_password.clone()} input_type="password" autocomplete="current-password" />
                <AuthInput id="new-password" label="Parola nouă" value={new_password.clone()} input_type="password" autocomplete="new-password" />
                <button class="secondary-button" type="submit" disabled={*busy}>{"Schimbă și revocă sesiunile"}</button>
            </form>

            <form class="security-card" onsubmit={update_profile}>
                <h3>{"Profil și cont"}</h3>
                <AuthInput id="profile-name" label="Nume afișat" value={display_name.clone()} input_type="text" autocomplete="name" />
                <button class="secondary-button" type="submit" disabled={*busy}>{"Salvează profilul"}</button>
                <button class="secondary-button danger-button" type="button" onclick={delete_account} disabled={*busy}>{"Șterge definitiv contul"}</button>
            </form>

            if let Some(text) = &*message {
                <div class="notice" role="status">{text}</div>
            }
        </div>
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct SessionSummary {
    id: Uuid,
    current: bool,
    user_agent: Option<String>,
    ip_address: Option<String>,
    expires_at: String,
}

#[derive(Debug, Deserialize)]
struct SessionListResponse {
    sessions: Vec<SessionSummary>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct PasskeySummary {
    id: Uuid,
    nickname: String,
}

#[derive(Debug, Deserialize)]
struct PasskeyListResponse {
    passkeys: Vec<PasskeySummary>,
    unused_backup_codes: i64,
}

#[derive(Debug, Deserialize)]
struct PasskeyRegistrationChallenge {
    ceremony_id: Uuid,
    public_key: Value,
}

#[derive(Debug, Deserialize)]
struct PasskeyRegistrationResponse {
    backup_codes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct BackupCodesResponse {
    backup_codes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct MeResponse {
    user: PublicUser,
}

enum LoginResult {
    Authenticated(AuthSession),
    MfaRequired(MfaChallenge),
}

async fn login(body: &Value) -> Result<LoginResult, String> {
    let request = Request::post("/api/v1/auth/login")
        .credentials(RequestCredentials::SameOrigin)
        .json(body)
        .map_err(|error| error.to_string())?;
    let response = request.send().await.map_err(|error| error.to_string())?;
    if response.status() == 202 {
        return parse_success::<MfaChallenge>(response)
            .await
            .map(LoginResult::MfaRequired);
    }
    parse_success::<AuthResponse>(response)
        .await
        .map(|response| LoginResult::Authenticated(response.into()))
}

async fn authenticate(path: &str, body: &Value) -> Result<LoginResult, String> {
    let request = Request::post(path)
        .credentials(RequestCredentials::SameOrigin)
        .json(body)
        .map_err(|error| error.to_string())?;
    let response = request.send().await.map_err(|error| error.to_string())?;
    parse_success::<AuthResponse>(response)
        .await
        .map(|response| LoginResult::Authenticated(response.into()))
}

async fn refresh_session(csrf_token: &str) -> Result<AuthSession, String> {
    let response = Request::post("/api/v1/auth/refresh")
        .credentials(RequestCredentials::SameOrigin)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    parse_success::<AuthResponse>(response)
        .await
        .map(Into::into)
}

async fn logout(csrf_token: &str) -> Result<(), String> {
    let response = Request::post("/api/v1/auth/logout")
        .credentials(RequestCredentials::SameOrigin)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    parse_empty(response).await
}

async fn finish_browser_mfa(challenge: &MfaChallenge) -> Result<AuthSession, String> {
    let options = serde_wasm_bindgen::to_value(&challenge.public_key)
        .map_err(|error| format!("Opțiuni WebAuthn invalide: {error}"))?;
    let credential = get_credential(options)
        .await
        .map_err(|error| js_error(&error))?;
    let credential: Value = serde_wasm_bindgen::from_value(credential)
        .map_err(|error| format!("Credential WebAuthn invalid: {error}"))?;
    let body = serde_json::json!({
        "ceremony_id": challenge.ceremony_id,
        "credential": credential,
    });
    match authenticate("/api/v1/auth/passkeys/login/finish", &body).await? {
        LoginResult::Authenticated(session) => Ok(session),
        LoginResult::MfaRequired(_) => Err("Răspuns MFA neașteptat.".to_owned()),
    }
}

async fn enroll_passkey(token: &str, body: &Value) -> Result<Vec<String>, String> {
    let start: PasskeyRegistrationChallenge =
        authorized_json("POST", "/api/v1/auth/passkeys/register/start", token, body).await?;
    let options = serde_wasm_bindgen::to_value(&start.public_key)
        .map_err(|error| format!("Opțiuni WebAuthn invalide: {error}"))?;
    let credential = create_credential(options)
        .await
        .map_err(|error| js_error(&error))?;
    let credential: Value = serde_wasm_bindgen::from_value(credential)
        .map_err(|error| format!("Credential WebAuthn invalid: {error}"))?;
    let finish = serde_json::json!({
        "ceremony_id": start.ceremony_id,
        "credential": credential,
    });
    let response: PasskeyRegistrationResponse = authorized_json(
        "POST",
        "/api/v1/auth/passkeys/register/finish",
        token,
        &finish,
    )
    .await?;
    Ok(response.backup_codes)
}

async fn reload_security(
    token: &str,
    sessions: &UseStateHandle<Vec<SessionSummary>>,
    passkeys: &UseStateHandle<Vec<PasskeySummary>>,
    backup_count: &UseStateHandle<i64>,
) -> Result<(), String> {
    let session_response: SessionListResponse =
        authorized_get("/api/v1/auth/sessions", token).await?;
    let passkey_response: PasskeyListResponse =
        authorized_get("/api/v1/auth/passkeys", token).await?;
    sessions.set(session_response.sessions);
    passkeys.set(passkey_response.passkeys);
    backup_count.set(passkey_response.unused_backup_codes);
    Ok(())
}

type SecurityReload = (
    UseStateHandle<Vec<SessionSummary>>,
    UseStateHandle<Vec<PasskeySummary>>,
    UseStateHandle<i64>,
);

fn action_without_body(
    token: String,
    path: &'static str,
    method: &'static str,
    success: &'static str,
    busy: UseStateHandle<bool>,
    message: UseStateHandle<Option<String>>,
    reload: Option<SecurityReload>,
) -> Callback<MouseEvent> {
    Callback::from(move |_| {
        busy.set(true);
        message.set(None);
        let token = token.clone();
        let busy = busy.clone();
        let message = message.clone();
        let reload = reload.clone();
        spawn_local(async move {
            match authorized_empty(method, path, &token).await {
                Ok(()) => {
                    if let Some((sessions, passkeys, backup_count)) = reload {
                        let _ = reload_security(&token, &sessions, &passkeys, &backup_count).await;
                    }
                    message.set(Some(success.to_owned()));
                }
                Err(error) => message.set(Some(error)),
            }
            busy.set(false);
        });
    })
}

async fn authorized_get<T: DeserializeOwned>(path: &str, token: &str) -> Result<T, String> {
    let response = Request::get(path)
        .credentials(RequestCredentials::SameOrigin)
        .header("authorization", &format!("Bearer {token}"))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    parse_success(response).await
}

async fn authorized_json<T: DeserializeOwned>(
    method: &str,
    path: &str,
    token: &str,
    body: &Value,
) -> Result<T, String> {
    let builder = match method {
        "PUT" => Request::put(path),
        "DELETE" => Request::delete(path),
        _ => Request::post(path),
    };
    let request = builder
        .credentials(RequestCredentials::SameOrigin)
        .header("authorization", &format!("Bearer {token}"))
        .json(body)
        .map_err(|error| error.to_string())?;
    let response = request.send().await.map_err(|error| error.to_string())?;
    parse_success(response).await
}

async fn authorized_empty(method: &str, path: &str, token: &str) -> Result<(), String> {
    let builder = match method {
        "DELETE" => Request::delete(path),
        _ => Request::post(path),
    };
    let response = builder
        .credentials(RequestCredentials::SameOrigin)
        .header("authorization", &format!("Bearer {token}"))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    parse_empty(response).await
}

async fn authorized_json_empty(
    method: &str,
    path: &str,
    token: &str,
    body: &Value,
) -> Result<(), String> {
    let builder = match method {
        "DELETE" => Request::delete(path),
        "PUT" => Request::put(path),
        _ => Request::post(path),
    };
    let request = builder
        .credentials(RequestCredentials::SameOrigin)
        .header("authorization", &format!("Bearer {token}"))
        .json(body)
        .map_err(|error| error.to_string())?;
    let response = request.send().await.map_err(|error| error.to_string())?;
    parse_empty(response).await
}

async fn parse_success<T: DeserializeOwned>(response: Response) -> Result<T, String> {
    if response.ok() {
        return response.json().await.map_err(|error| error.to_string());
    }
    Err(parse_error(response).await)
}

async fn parse_empty(response: Response) -> Result<(), String> {
    if response.ok() {
        Ok(())
    } else {
        Err(parse_error(response).await)
    }
}

async fn parse_error(response: Response) -> String {
    response.json::<ErrorEnvelope>().await.map_or_else(
        |_| format!("Cererea a eșuat cu status {}.", response.status()),
        |error| error.error.message,
    )
}

fn set_authenticated(callback: &Callback<Option<AuthSession>>, session: AuthSession) {
    store_csrf(&session.csrf_token);
    callback.emit(Some(session));
}

fn clear_auth(callback: &Callback<Option<AuthSession>>) {
    if let Some(storage) = session_storage() {
        let _ = storage.remove_item(CSRF_STORAGE_KEY);
    }
    callback.emit(None);
}

fn load_csrf() -> Option<String> {
    session_storage()?.get_item(CSRF_STORAGE_KEY).ok().flatten()
}

fn store_csrf(value: &str) {
    if let Some(storage) = session_storage() {
        let _ = storage.set_item(CSRF_STORAGE_KEY, value);
    }
}

fn session_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.session_storage().ok().flatten()
}

fn js_error(value: &JsValue) -> String {
    value
        .as_string()
        .or_else(|| {
            js_sys::Reflect::get(value, &JsValue::from_str("message"))
                .ok()
                .and_then(|message| message.as_string())
        })
        .unwrap_or_else(|| "Operația WebAuthn a fost anulată sau refuzată.".to_owned())
}
