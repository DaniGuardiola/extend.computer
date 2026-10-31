//! Native passkey sign-in through AuthenticationServices.
//!
//! macOS only lets an app use passkeys for a relying party it is associated
//! with: the app needs the `webcredentials:` Associated Domains entitlement,
//! backed by an embedded provisioning profile, and the domain must list the
//! app in its apple-app-site-association file. Builds without the profile
//! (development builds, custom account servers) use browser sign-in instead.
//!
//! Each request offers both passkeys (iCloud Keychain, password managers,
//! nearby phones) and hardware security keys, like a browser does.
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass, MainThreadOnly};
use objc2_app_kit::NSWindow;
use objc2_authentication_services::{
    ASAuthorization, ASAuthorizationAllSupportedPublicKeyCredentialDescriptorTransports,
    ASAuthorizationController, ASAuthorizationControllerDelegate,
    ASAuthorizationControllerPresentationContextProviding,
    ASAuthorizationPlatformPublicKeyCredentialAssertion,
    ASAuthorizationPlatformPublicKeyCredentialDescriptor,
    ASAuthorizationPlatformPublicKeyCredentialProvider,
    ASAuthorizationPublicKeyCredentialAssertion,
    ASAuthorizationPublicKeyCredentialAssertionRequest,
    ASAuthorizationPublicKeyCredentialUserVerificationPreferencePreferred,
    ASAuthorizationPublicKeyCredentialUserVerificationPreferenceRequired, ASAuthorizationRequest,
    ASAuthorizationSecurityKeyPublicKeyCredentialAssertion,
    ASAuthorizationSecurityKeyPublicKeyCredentialDescriptor,
    ASAuthorizationSecurityKeyPublicKeyCredentialProvider, ASPublicKeyCredential,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSData, NSError, NSString};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::sync::mpsc;
use tauri::Manager;

use crate::accounts::{Ceremony, RELYING_PARTY};

/// Only builds signed with the Associated Domains profile can use passkeys;
/// asking without it fails, so offer browser sign-in instead.
pub fn available() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.parent()?
                .parent()
                .map(|contents| contents.join("embedded.provisionprofile"))
        })
        .is_some_and(|profile| profile.is_file())
}

type Reply = mpsc::Sender<Result<Value, String>>;

struct Ivars {
    window: Retained<NSWindow>,
    reply: RefCell<Option<Reply>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ExtendComputerPasskeyDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl ASAuthorizationControllerDelegate for Delegate {
        #[unsafe(method(authorizationController:didCompleteWithAuthorization:))]
        fn completed(&self, _: &ASAuthorizationController, authorization: &ASAuthorization) {
            self.finish(credential(authorization));
        }

        #[unsafe(method(authorizationController:didCompleteWithError:))]
        fn failed(&self, _: &ASAuthorizationController, error: &NSError) {
            // Code 1001 is ASAuthorizationErrorCanceled: the user dismissed the sheet.
            self.finish(Err(if error.code() == 1001 {
                "Passkey sign-in was canceled.".into()
            } else {
                error.localizedDescription().to_string()
            }));
        }
    }

    unsafe impl ASAuthorizationControllerPresentationContextProviding for Delegate {
        #[unsafe(method_id(presentationAnchorForAuthorizationController:))]
        fn anchor(&self, _: &ASAuthorizationController) -> Retained<NSWindow> {
            self.ivars().window.clone()
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker, window: Retained<NSWindow>, reply: Reply) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            window,
            reply: RefCell::new(Some(reply)),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn finish(&self, result: Result<Value, String>) {
        if let Some(reply) = self.ivars().reply.borrow_mut().take() {
            let _ = reply.send(result);
        }
        // The controller and delegate stay alive until a reply is sent.
        ACTIVE.with(|active| active.borrow_mut().take());
    }
}

thread_local! {
    static ACTIVE: RefCell<Option<(Retained<ASAuthorizationController>, Retained<Delegate>)>> =
        const { RefCell::new(None) };
}

fn base64url(data: &NSData) -> String {
    URL_SAFE_NO_PAD.encode(data.to_vec())
}

fn assertion_json(
    assertion: &ProtocolObject<dyn ASAuthorizationPublicKeyCredentialAssertion>,
    attachment: &str,
) -> Value {
    let id = unsafe { base64url(&assertion.credentialID()) };
    let user = unsafe { assertion.userID() };
    let mut response = unsafe {
        json!({
            "authenticatorData": base64url(&assertion.rawAuthenticatorData()),
            "clientDataJSON": base64url(&assertion.rawClientDataJSON()),
            "signature": base64url(&assertion.signature()),
        })
    };
    // Security keys used as a second factor need not store a user handle.
    if !user.is_empty() {
        response["userHandle"] = base64url(&user).into();
    }
    json!({
        "id": id,
        "rawId": id,
        "type": "public-key",
        "authenticatorAttachment": attachment,
        "clientExtensionResults": {},
        "response": response,
    })
}

/// Shape the assertion like a browser's PublicKeyCredential JSON, which the
/// account servers already verify.
fn credential(authorization: &ASAuthorization) -> Result<Value, String> {
    let credential = unsafe { authorization.credential() };
    let object: &AnyObject = credential.as_ref();
    if let Some(passkey) =
        object.downcast_ref::<ASAuthorizationPlatformPublicKeyCredentialAssertion>()
    {
        return Ok(assertion_json(
            ProtocolObject::from_ref(passkey),
            "platform",
        ));
    }
    if let Some(key) =
        object.downcast_ref::<ASAuthorizationSecurityKeyPublicKeyCredentialAssertion>()
    {
        return Ok(assertion_json(
            ProtocolObject::from_ref(key),
            "cross-platform",
        ));
    }
    Err("The passkey response was not a sign-in assertion.".into())
}

fn decode(value: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(value.trim_end_matches('=')).ok()
}

/// Ask the user to sign in with a passkey or security key for
/// `RELYING_PARTY`, answering the server's challenge. Blocks until the system
/// sheet completes or is closed. Second factors only need presence;
/// first-factor passkeys must verify the user with a PIN or biometrics.
pub fn sign_in(app: &tauri::AppHandle, request: &Ceremony, verify_user: bool) -> Result<Value> {
    let challenge = decode(&request.challenge)
        .context("The account server sent an invalid passkey challenge.")?;
    let allowed = request
        .allowed
        .iter()
        .map(|id| decode(id))
        .collect::<Option<Vec<_>>>()
        .context("The account server sent an invalid credential.")?;
    let window = app
        .get_webview_window("main")
        .context("The app window is not available.")?;
    let (reply, result) = mpsc::channel();
    let main = window.clone();
    main.run_on_main_thread(move || {
        let Some(mtm) = MainThreadMarker::new() else {
            let _ = reply.send(Err("Passkeys must start on the main thread.".into()));
            return;
        };
        let Ok(handle) = window.ns_window() else {
            let _ = reply.send(Err("The app window is not available.".into()));
            return;
        };
        unsafe {
            let Some(native) = Retained::retain(handle.cast::<NSWindow>()) else {
                let _ = reply.send(Err("The app window is not available.".into()));
                return;
            };
            let rp = NSString::from_str(RELYING_PARTY);
            let challenge = NSData::with_bytes(&challenge);
            let preference = if verify_user {
                ASAuthorizationPublicKeyCredentialUserVerificationPreferenceRequired
            } else {
                ASAuthorizationPublicKeyCredentialUserVerificationPreferencePreferred
            };
            let passkeys = ASAuthorizationPlatformPublicKeyCredentialProvider::initWithRelyingPartyIdentifier(
                ASAuthorizationPlatformPublicKeyCredentialProvider::alloc(),
                &rp,
            )
            .createCredentialAssertionRequestWithChallenge(&challenge);
            let keys = ASAuthorizationSecurityKeyPublicKeyCredentialProvider::initWithRelyingPartyIdentifier(
                ASAuthorizationSecurityKeyPublicKeyCredentialProvider::alloc(),
                &rp,
            )
            .createCredentialAssertionRequestWithChallenge(&challenge);
            if let Some(preference) = preference {
                passkeys.setUserVerificationPreference(preference);
                keys.setUserVerificationPreference(preference);
            }
            if !allowed.is_empty() {
                let transports = ASAuthorizationAllSupportedPublicKeyCredentialDescriptorTransports();
                let platform: Vec<_> = allowed
                    .iter()
                    .map(|id| {
                        ASAuthorizationPlatformPublicKeyCredentialDescriptor::initWithCredentialID(
                            ASAuthorizationPlatformPublicKeyCredentialDescriptor::alloc(),
                            &NSData::with_bytes(id),
                        )
                    })
                    .collect();
                let hardware: Vec<_> = allowed
                    .iter()
                    .map(|id| {
                        ASAuthorizationSecurityKeyPublicKeyCredentialDescriptor::initWithCredentialID_transports(
                            ASAuthorizationSecurityKeyPublicKeyCredentialDescriptor::alloc(),
                            &NSData::with_bytes(id),
                            &transports,
                        )
                    })
                    .collect();
                passkeys.setAllowedCredentials(&NSArray::from_retained_slice(&platform));
                keys.setAllowedCredentials(&NSArray::from_retained_slice(&hardware));
            }
            let requests = NSArray::from_retained_slice(&[
                Retained::into_super(passkeys),
                Retained::into_super(keys),
            ]);
            let controller = ASAuthorizationController::initWithAuthorizationRequests(
                ASAuthorizationController::alloc(),
                &requests as &NSArray<ASAuthorizationRequest>,
            );
            let delegate = Delegate::new(mtm, native, reply);
            controller.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            controller.setPresentationContextProvider(Some(ProtocolObject::from_ref(&*delegate)));
            ACTIVE.with(|active| *active.borrow_mut() = Some((controller.clone(), delegate)));
            controller.performRequests();
        }
    })?;
    match result.recv() {
        Ok(Ok(credential)) => Ok(credential),
        Ok(Err(message)) => bail!(message),
        Err(_) => bail!("Passkey sign-in stopped unexpectedly."),
    }
}
