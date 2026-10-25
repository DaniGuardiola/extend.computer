//! Native passkey sign-in through AuthenticationServices.
//!
//! macOS only lets an app use passkeys for a relying party it is associated
//! with: the app needs the `webcredentials:` Associated Domains entitlement,
//! backed by an embedded provisioning profile, and the domain must list the
//! app in its apple-app-site-association file. Builds without the profile
//! (development builds, custom account servers) use browser sign-in instead.
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass, MainThreadOnly};
use objc2_app_kit::NSWindow;
use objc2_authentication_services::{
    ASAuthorization, ASAuthorizationController, ASAuthorizationControllerDelegate,
    ASAuthorizationControllerPresentationContextProviding,
    ASAuthorizationPlatformPublicKeyCredentialAssertion,
    ASAuthorizationPlatformPublicKeyCredentialProvider,
    ASAuthorizationPublicKeyCredentialAssertion, ASAuthorizationRequest, ASPublicKeyCredential,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSData, NSError, NSString};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::sync::mpsc;
use tauri::Manager;

/// The relying party the official account server registers passkeys under.
pub const RELYING_PARTY: &str = "extend.computer";

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

/// Shape the assertion like a browser's PublicKeyCredential JSON, which the
/// account servers already verify.
fn credential(authorization: &ASAuthorization) -> Result<Value, String> {
    let credential = unsafe { authorization.credential() };
    let object: &AnyObject = credential.as_ref();
    let Some(assertion) =
        object.downcast_ref::<ASAuthorizationPlatformPublicKeyCredentialAssertion>()
    else {
        return Err("The passkey response was not a sign-in assertion.".into());
    };
    let id = unsafe { base64url(&assertion.credentialID()) };
    Ok(unsafe {
        json!({
            "id": id,
            "rawId": id,
            "type": "public-key",
            "authenticatorAttachment": "platform",
            "clientExtensionResults": {},
            "response": {
                "authenticatorData": base64url(&assertion.rawAuthenticatorData()),
                "clientDataJSON": base64url(&assertion.rawClientDataJSON()),
                "signature": base64url(&assertion.signature()),
                "userHandle": base64url(&assertion.userID()),
            },
        })
    })
}

/// Ask the user to sign in with a passkey for `RELYING_PARTY`, answering the
/// server's challenge. Blocks until the system sheet completes or is closed.
pub fn sign_in(app: &tauri::AppHandle, challenge: &str) -> Result<Value> {
    let challenge = URL_SAFE_NO_PAD
        .decode(challenge.trim_end_matches('='))
        .context("The account server sent an invalid passkey challenge.")?;
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
            let provider =
                ASAuthorizationPlatformPublicKeyCredentialProvider::initWithRelyingPartyIdentifier(
                    ASAuthorizationPlatformPublicKeyCredentialProvider::alloc(),
                    &NSString::from_str(RELYING_PARTY),
                );
            let request = provider
                .createCredentialAssertionRequestWithChallenge(&NSData::with_bytes(&challenge));
            let requests = NSArray::from_retained_slice(&[Retained::into_super(request)]);
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
