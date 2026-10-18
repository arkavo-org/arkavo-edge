import CryptoKit
import Foundation
import Security

// C entry points for the Rust crate. CryptoKit is the only public API that
// hands back an enclave key as a wrapped blob the caller can keep in a file;
// Security.framework keeps enclave keys only in the data-protection
// keychain, which a command-line binary cannot use without an entitlement.
//
// Status codes: 0 ok, 1 no Secure Enclave, 2 the enclave refused,
// 3 the caller's buffer is too small.

@_cdecl("arkavo_se_available")
public func arkavoSeAvailable() -> Bool {
    SecureEnclave.isAvailable
}

@_cdecl("arkavo_se_create")
public func arkavoSeCreate(
    _ blob: UnsafeMutablePointer<UInt8>, _ capacity: Int, _ blobLength: UnsafeMutablePointer<Int>
) -> Int32 {
    guard SecureEnclave.isAvailable else { return 1 }
    // Private-key usage alone: no user presence, biometry or passcode, so an
    // agent signs unattended. After first unlock keeps the key usable while
    // the screen is locked, once someone has logged in since boot.
    guard let access = SecAccessControlCreateWithFlags(
        nil, kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly, .privateKeyUsage, nil)
    else { return 2 }
    guard let key = try? SecureEnclave.P256.Signing.PrivateKey(accessControl: access) else {
        return 2
    }
    let wrapped = key.dataRepresentation
    guard wrapped.count <= capacity else { return 3 }
    wrapped.copyBytes(to: blob, count: wrapped.count)
    blobLength.pointee = wrapped.count
    return 0
}

@_cdecl("arkavo_se_public_key")
public func arkavoSePublicKey(
    _ blob: UnsafePointer<UInt8>, _ length: Int, _ point: UnsafeMutablePointer<UInt8>
) -> Int32 {
    guard let key = try? SecureEnclave.P256.Signing.PrivateKey(
        dataRepresentation: Data(bytes: blob, count: length))
    else { return 2 }
    // x963 is the 65-byte uncompressed point; compressedRepresentation would
    // need macOS 13.
    key.publicKey.x963Representation.copyBytes(to: point, count: 65)
    return 0
}

@_cdecl("arkavo_se_sign")
public func arkavoSeSign(
    _ blob: UnsafePointer<UInt8>, _ length: Int,
    _ message: UnsafePointer<UInt8>, _ messageLength: Int,
    _ signature: UnsafeMutablePointer<UInt8>
) -> Int32 {
    guard let key = try? SecureEnclave.P256.Signing.PrivateKey(
            dataRepresentation: Data(bytes: blob, count: length)),
        let signed = try? key.signature(for: Data(bytes: message, count: messageLength))
    else { return 2 }
    signed.rawRepresentation.copyBytes(to: signature, count: 64)
    return 0
}
