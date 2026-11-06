#if canImport(UIKit)
import UIKit

/// API-only sign-in fallback (port of Android's AccountLoginDialog): email and
/// password, then an authenticator or recovery code when the account has 2FA.
/// Passwords, MFA tickets and codes never cross into React Native. Security
/// keys and passkeys need browser sign-in.
final class AccountLoginViewController: UIViewController, UIAdaptivePresentationControllerDelegate, UITextFieldDelegate {
  private let accounts: AccountsController
  private let email = UITextField(), password = UITextField(), code = UITextField()
  private let message = UILabel()
  private lazy var submitButton = UIBarButtonItem(title: "Sign in", style: .done, target: self, action: #selector(submit))
  private(set) lazy var container: UINavigationController = {
    let navigation = UINavigationController(rootViewController: self)
    navigation.modalPresentationStyle = .formSheet
    navigation.presentationController?.delegate = self
    return navigation
  }()
  private var generation: Int64 = -1
  private var ticket: String?
  private var busy = false
  private var finished = false

  init(accounts: AccountsController) {
    self.accounts = accounts
    super.init(nibName: nil, bundle: nil)
  }
  required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

  /// Starts the native sign-in generation; the controller can dismiss this form
  /// (cancel, timeout, logout) through the closure it receives.
  func open() throws {
    generation = try accounts.beginNative { [weak self] in
      DispatchQueue.main.async { self?.close() }
    }
  }

  override func viewDidLoad() {
    super.viewDidLoad()
    title = "Sign in to your account"
    view.backgroundColor = .systemBackground
    navigationItem.leftBarButtonItem = UIBarButtonItem(barButtonSystemItem: .cancel, target: self, action: #selector(cancel))
    navigationItem.rightBarButtonItem = submitButton
    func field(_ field: UITextField, _ placeholder: String, _ label: String) {
      field.placeholder = placeholder
      field.accessibilityLabel = label
      field.borderStyle = .roundedRect
      field.autocapitalizationType = .none
      field.autocorrectionType = .no
      field.spellCheckingType = .no
      field.delegate = self
      field.font = .preferredFont(forTextStyle: .body)
      field.adjustsFontForContentSizeCategory = true
    }
    field(email, "Email", "Account email")
    email.keyboardType = .emailAddress
    email.textContentType = .username
    email.returnKeyType = .next
    field(password, "Password", "Account password")
    password.isSecureTextEntry = true
    password.textContentType = .password
    password.returnKeyType = .go
    field(code, "Authenticator or recovery code", "Authenticator or recovery code")
    code.textContentType = .oneTimeCode
    code.returnKeyType = .go
    code.isHidden = true
    message.text = "Sign in on this device. Use browser sign-in for passkeys or security keys."
    message.numberOfLines = 0
    message.font = .preferredFont(forTextStyle: .footnote)
    message.adjustsFontForContentSizeCategory = true
    message.textColor = .secondaryLabel
    let stack = UIStackView(arrangedSubviews: [email, password, code, message])
    stack.axis = .vertical
    stack.spacing = 12
    stack.translatesAutoresizingMaskIntoConstraints = false
    view.addSubview(stack)
    NSLayoutConstraint.activate([
      stack.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 24),
      stack.leadingAnchor.constraint(equalTo: view.layoutMarginsGuide.leadingAnchor),
      stack.trailingAnchor.constraint(equalTo: view.layoutMarginsGuide.trailingAnchor),
    ])
    if let sheet = container.sheetPresentationController { sheet.detents = [.medium(), .large()] }
  }

  override func viewDidAppear(_ animated: Bool) {
    super.viewDidAppear(animated)
    if !finished { email.becomeFirstResponder() }
  }

  func textFieldShouldReturn(_ field: UITextField) -> Bool {
    if field === email { password.becomeFirstResponder() } else { submit() }
    return false
  }

  private func say(_ text: String) {
    message.text = text
    UIAccessibility.post(notification: .announcement, argument: text)
  }

  @objc private func cancel() { close() }

  /// Dismisses and wipes the form; tells the controller the flow ended.
  private func close() {
    guard !finished else { return }
    finished = true
    wipe()
    accounts.nativeClosed(generation)
    if container.presentingViewController != nil { container.dismiss(animated: true) }
  }

  private func wipe() {
    email.text = nil; password.text = nil; code.text = nil; ticket = nil
  }

  // Swipe-down dismissal.
  func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
    guard !finished else { return }
    finished = true
    wipe()
    accounts.nativeClosed(generation)
  }

  @objc func submit() {
    guard !busy, !finished else { return }
    let enteredEmail = (email.text ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    let enteredPassword = password.text ?? ""
    let enteredCode = (code.text ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    let pending = ticket
    if pending == nil && (enteredEmail.isEmpty || enteredPassword.isEmpty) { say("Enter your email and password."); return }
    if pending != nil && enteredCode.isEmpty { say("Enter your authenticator or recovery code."); return }
    password.text = nil
    code.text = nil
    busy = true
    submitButton.isEnabled = false
    let generation = generation, accounts = accounts
    do {
      try accounts.submitNative { [weak self] in
        var response: [String: Any]?
        var failure: String?
        do {
          response = try pending == nil
            ? accounts.password(generation, email: enteredEmail, password: enteredPassword)
            : accounts.verify(generation, secondFactor: pending!, code: enteredCode)
        } catch let failed as AccountHTTPFailure {
          failure = failed.message
        } catch {
          failure = "Couldn't sign in. Check your account details and try again."
        }
        DispatchQueue.main.async { self?.finish(response, failure) }
      }
    } catch {
      busy = false
      submitButton.isEnabled = true
      say("Couldn't sign in. Return to the app and retry.")
    }
  }

  private func finish(_ response: [String: Any]?, _ failure: String?) {
    guard !finished else { return }
    busy = false
    submitButton.isEnabled = true
    if let failure { say(failure); return }
    guard let response, response["mfa_required"] as? Bool == true else {
      finished = true
      wipe()
      container.dismiss(animated: true)
      return
    }
    ticket = response["ticket"] as? String
    email.isHidden = true
    password.isHidden = true
    code.isHidden = false
    code.becomeFirstResponder()
    submitButton.title = "Verify"
    let codes = response["totp"] as? Bool == true || response["recovery"] as? Bool == true
    say(codes ? "Enter an authenticator or recovery code." : "This account requires a security key. Cancel and use browser sign-in.")
  }
}
#endif
