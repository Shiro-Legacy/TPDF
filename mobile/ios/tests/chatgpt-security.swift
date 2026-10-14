// Appended to the exact production parser/verifier declarations by run-chatgpt-security.mjs.
// No network requests: the async HTTP helper always fails in this harness.
private var checks = 0
private func check(_ condition: Bool, _ name: String) {
    guard condition else { fatalError("FAIL: " + name) }; checks += 1
}
private func rejects(_ name: String, _ operation: () throws -> Void) {
    do { try operation(); fatalError("FAIL: accepted " + name) }
    catch { checks += 1 }
}
private func encoded(_ value: [String: Any]) throws -> String { try JSONSerialization.data(withJSONObject: value, options: .sortedKeys).base64URL }
private func signed(_ claims: [String: Any], _ key: SecKey, header: [String: Any] = ["alg": "RS256", "kid": "test-key"]) throws -> String {
    let input = try encoded(header) + "." + encoded(claims)
    guard let signature = SecKeyCreateSignature(key, .rsaSignatureMessagePKCS1v15SHA256, Data(input.utf8) as CFData, nil) as Data? else { fatalError("Cannot sign fixture") }
    return input + "." + signature.base64URL
}
private func derElement(_ bytes: [UInt8], _ cursor: inout Int, tag: UInt8) -> [UInt8] {
    guard bytes[cursor] == tag else { fatalError("Unexpected public-key encoding") }; cursor += 1
    var length = Int(bytes[cursor]); cursor += 1
    if length & 128 != 0 {
        let octets = length & 127; length = 0
        for _ in 0..<octets { length = (length << 8) | Int(bytes[cursor]); cursor += 1 }
    }
    let result = Array(bytes[cursor..<cursor+length]); cursor += length; return result
}
private func verifyFixture(_ token: String, _ jwks: [String: Any], audience: String = "oaiapp_test", nonce: String? = "test-nonce", authorizedParty: String? = nil) throws -> [String: Any] {
    try ChatGPTJWT.verify(token, audience: audience, nonce: nonce, authorizedParty: authorizedParty, jwks: jwks)
}
var keyError: Unmanaged<CFError>?
guard let privateKey = SecKeyCreateRandomKey([kSecAttrKeyType: kSecAttrKeyTypeRSA, kSecAttrKeySizeInBits: 2048, kSecPrivateKeyAttrs: [kSecAttrIsPermanent: false]] as CFDictionary, &keyError) else { fatalError("RSA test key creation failed: \(String(describing: keyError?.takeRetainedValue()))") }
let publicKey = SecKeyCopyPublicKey(privateKey)!
let publicBytes = Array(SecKeyCopyExternalRepresentation(publicKey, nil)! as Data)
var cursor = 0
let sequence = derElement(publicBytes, &cursor, tag: 0x30)
cursor = 0
var modulus = derElement(sequence, &cursor, tag: 0x02)
while modulus.count > 1 && modulus.first == 0 { modulus.removeFirst() }
let exponent = derElement(sequence, &cursor, tag: 0x02)
let jwk: [String: Any] = ["kty": "RSA", "kid": "test-key", "alg": "RS256", "use": "sig", "n": Data(modulus).base64URL, "e": Data(exponent).base64URL]
let jwks: [String: Any] = ["keys": [jwk]]
let now = Date().timeIntervalSince1970
let claims: [String: Any] = ["iss": "https://auth.openai.com", "aud": "oaiapp_test", "sub": "account-test", "nonce": "test-nonce", "exp": now + 3600]
let valid = try signed(claims, privateKey)
check(try verifyFixture(valid, jwks)["sub"] as? String == "account-test", "valid RSA token")
for (field, value) in [("nonce", "wrong" as Any), ("aud", "oaiapp_other" as Any), ("iss", "https://evil.example" as Any), ("exp", now - 1000 as Any), ("nbf", now + 1000 as Any), ("sub", "" as Any), ("azp", "oaiapp_other" as Any)] {
    var changed = claims; changed[field] = value
    let token = try signed(changed, privateKey)
    rejects("JWT " + field) { _ = try verifyFixture(token, jwks) }
}
var tampered = claims; tampered["sub"] = "different-account"
let pieces = valid.split(separator: ".")
let forged = String(pieces[0]) + "." + (try encoded(tampered)) + "." + String(pieces[2])
rejects("tampered signature") { _ = try verifyFixture(forged, jwks) }
rejects("duplicate signing key") { _ = try verifyFixture(valid, ["keys": [jwk, jwk]]) }
rejects("unknown signing key") { _ = try verifyFixture(valid, ["keys": []]) }
let wrongAlgorithm = try signed(claims, privateKey, header: ["alg": "HS256", "kid": "test-key"])
rejects("algorithm confusion") { _ = try verifyFixture(wrongAlgorithm, jwks) }
var multi = claims; multi["aud"] = ["https://api.openai.com/v1", "other-resource"]; multi["azp"] = "oaiapp_test"; multi.removeValue(forKey: "nonce")
let access = try signed(multi, privateKey)
check(try verifyFixture(access, jwks, audience: "https://api.openai.com/v1", nonce: nil, authorizedParty: "oaiapp_test")["sub"] as? String == "account-test", "multi-audience client binding")
rejects("wrong authorized party") { _ = try verifyFixture(access, jwks, audience: "https://api.openai.com/v1", nonce: nil, authorizedParty: "another-client") }

let callback = "GET /auth/callback?code=one-use-code&state=expected&client_id=oaiapp_test HTTP/1.1\r\nHost: 127.0.0.1:12345\r\n\r\n"
check(ChatGPTOAuthCallback.parse(callback, state: "expected", port: 12345)?["code"] == "one-use-code", "valid loopback callback")
for invalid in [
    callback.replacingOccurrences(of: "state=expected", with: "state=wrong"),
    callback.replacingOccurrences(of: "state=expected", with: "state=expected&state=expected"),
    callback.replacingOccurrences(of: "state=expected", with: "state=expected&%73tate=expected"),
    callback.replacingOccurrences(of: "Host: 127.0.0.1:12345", with: "Host: evil.example"),
    callback.replacingOccurrences(of: "Host: 127.0.0.1:12345", with: "Host: 127.0.0.1:12345\r\nHost: evil.example"),
    callback.replacingOccurrences(of: "GET ", with: "POST "),
    callback.replacingOccurrences(of: "/auth/callback?", with: "/wrong?"),
    callback.replacingOccurrences(of: "/auth/callback?", with: "https://evil.example/auth/callback?"),
    callback.replacingOccurrences(of: " HTTP/1.1", with: "#fragment HTTP/1.1"),
    callback + String(repeating: "x", count: 20_000)
] { check(ChatGPTOAuthCallback.parse(invalid, state: "expected", port: 12345) == nil, "invalid loopback callback") }

func event(_ value: [String: Any]) throws -> Data {
    let json = String(data: try JSONSerialization.data(withJSONObject: value), encoding: .utf8)!
    return Data(("event: ignored\ndata: " + json + "\n\n").utf8)
}
func complete(_ response: [String: Any]) throws -> Data { try event(["type": "response.completed", "response": response]) }
let output: [[String: Any]] = [["type": "reasoning", "encrypted_content": "OPAQUE_SECRET"], ["type": "message", "status": "completed", "content": [["type": "output_text", "text": "# Real document\nTotal 18.75", "annotations": ["PRIVATE_METADATA"]]]]]
let response: [String: Any] = ["status": "completed", "output": output, "metadata": ["token": "OPAQUE_SECRET"], "service_tier": "private"]
let stream = try complete(response)
let clean = try ChatGPTStream.completed(stream)
let serialized = String(data: try JSONSerialization.data(withJSONObject: clean), encoding: .utf8)!
check(serialized.contains("Real document") && !serialized.contains("OPAQUE_SECRET") && !serialized.contains("PRIVATE_METADATA") && !serialized.contains("service_tier"), "bridge only contains final document text")
rejects("missing terminal event") { _ = try ChatGPTStream.completed(event(["type": "response.output_text.delta", "delta": "partial"])) }
rejects("truncated event JSON") { _ = try ChatGPTStream.completed(Data("data: {\"type\":\"response.completed\"".utf8)) }
rejects("DONE without completion") { _ = try ChatGPTStream.completed(Data("data: [DONE]\n\n".utf8)) }
rejects("duplicate completion") { _ = try ChatGPTStream.completed(stream + stream) }
rejects("terminal failure after text") { _ = try ChatGPTStream.completed(stream + event(["type": "response.failed"])) }
for status in ["incomplete", "failed"] {
    var changed = response; changed["status"] = status
    rejects("noncomplete response") { _ = try ChatGPTStream.completed(complete(changed)) }
}
var incomplete = response; incomplete["incomplete_details"] = ["reason": "max_output_tokens"]
rejects("inconsistent incomplete details") { _ = try ChatGPTStream.completed(complete(incomplete)) }
let badParts: [[String: Any]] = [["type": "refusal", "refusal": "no"], ["type": "output_text", "text": String(repeating: "x", count: 24_001)], ["type": "output_text", "text": "  "]]
for part in badParts {
    let bad: [String: Any] = ["status": "completed", "output": [["type": "message", "content": [part]]]]
    rejects("unsafe or unusable document content") { _ = try ChatGPTStream.completed(complete(bad)) }
}
print("PASS: \(checks) iOS OAuth callback, RSA JWT, and terminal stream security checks (actual production functions; no network).")
