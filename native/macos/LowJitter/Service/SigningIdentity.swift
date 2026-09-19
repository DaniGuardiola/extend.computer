import Foundation
import Security
import CryptoKit

/// Trust comes from the executing signed image, never from caller-supplied data.
struct ServiceIdentity {
    let development: Bool
    let requirement: String
    var name: String { development ? "computer.extend.lowjitter.development" : "computer.extend.lowjitter" }
    var brokerIdentifier: String { development ? "computer.extend.lowjitter.broker.development" : "computer.extend.lowjitter.broker" }
    enum Failure: Error { case unsignedBuild, unexpectedIdentity, invalidSignature }

    static func current(application: Bool = false) throws -> ServiceIdentity {
        var code: SecCode?
        guard SecCodeCopySelf([], &code) == errSecSuccess, let code else { throw Failure.invalidSignature }
        var staticCode: SecStaticCode?
        guard SecCodeCopyStaticCode(code, [], &staticCode) == errSecSuccess, let staticCode else { throw Failure.invalidSignature }
        var information: CFDictionary?
        guard SecCodeCopySigningInformation(staticCode, SecCSFlags(rawValue: kSecCSSigningInformation), &information) == errSecSuccess,
              let info = information as? [String: Any],
              let identifier = info[kSecCodeInfoIdentifier as String] as? String,
              let certificates = info[kSecCodeInfoCertificates as String] as? [SecCertificate],
              let certificate = certificates.first else { throw Failure.unsignedBuild }
        let base = application ? "computer.extend.desktop" : "computer.extend.lowjitter.broker"
        guard identifier == base || identifier == base + ".development" else { throw Failure.unexpectedIdentity }
        let development = identifier.hasSuffix(".development")
        let broker = "computer.extend.lowjitter.broker" + (development ? ".development" : "")
        let signer: String
        if development {
            let digest = Insecure.SHA1.hash(data: SecCertificateCopyData(certificate) as Data)
                .map { String(format: "%02x", $0) }.joined()
            signer = "certificate leaf = H\"\(digest)\""
        } else {
            guard let team = info[kSecCodeInfoTeamIdentifier as String] as? String,
                  team.range(of: "^[A-Z0-9]{10}$", options: .regularExpression) != nil else { throw Failure.invalidSignature }
            signer = "anchor apple generic and certificate leaf[subject.OU] = \"\(team)\""
        }
        return ServiceIdentity(development: development, requirement: "identifier \"\(broker)\" and \(signer)")
    }

    func verifyBroker(at url: URL) throws {
        var code: SecStaticCode?
        var requirement: SecRequirement?
        guard SecStaticCodeCreateWithPath(url as CFURL, [], &code) == errSecSuccess,
              let code,
              SecRequirementCreateWithString(self.requirement as CFString, [], &requirement) == errSecSuccess,
              SecStaticCodeCheckValidity(code, SecCSFlags(rawValue: kSecCSStrictValidate), requirement) == errSecSuccess else {
            throw Failure.invalidSignature
        }
    }
}
