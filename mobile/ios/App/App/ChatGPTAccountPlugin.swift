import Capacitor
import AuthenticationServices
import CryptoKit
import Foundation
import Network
import Security
import UIKit

// OAuth credentials never cross the Capacitor bridge. Only the account summary,
// model catalog and a validated, text-only final response are exposed to JS.
@objc(ChatGPTAccountPlugin)
public final class ChatGPTAccountPlugin: CAPPlugin, CAPBridgedPlugin {
    public let identifier = "ChatGPTAccountPlugin"
    public let jsName = "ChatGPTAccount"
    public let pluginMethods: [CAPPluginMethod] = ["getAccounts", "signIn", "cancelSignIn", "signOut", "selectAccount", "models", "reconstruct", "manageUsage"].compactMap { CAPPluginMethod(name: $0, returnType: CAPPluginReturnPromise) }
    private var service: ChatGPTAccountService?
    private func perform(_ call: CAPPluginCall, operation: @escaping @MainActor (ChatGPTAccountService) async throws -> [String: Any]) {
        Task { @MainActor in
            do {
                if self.service == nil { self.service = try ChatGPTAccountService() }
                guard let service = self.service else { throw ChatGPTFailure.storage }
                service.window = self.bridge?.viewController?.view.window
                call.resolve(try await operation(service))
            } catch let error as ChatGPTFailure { call.reject(error.message) }
            catch is CancellationError { call.reject(ChatGPTFailure.cancelled.message) }
            catch { call.reject(ChatGPTFailure.connection.message) }
        }
    }
    @objc func getAccounts(_ call: CAPPluginCall) { perform(call) { $0.snapshot() } }
    @objc func signIn(_ call: CAPPluginCall) { perform(call) { try await $0.signIn(accountID: call.getString("accountId")) } }
    @objc func cancelSignIn(_ call: CAPPluginCall) { perform(call) { $0.cancelSignIn(); return [:] } }
    @objc func signOut(_ call: CAPPluginCall) { perform(call) { try await $0.signOut(accountID: call.getString("accountId") ?? "") } }
    @objc func selectAccount(_ call: CAPPluginCall) { perform(call) { try $0.select(accountID: call.getString("accountId") ?? "") } }
    @objc func models(_ call: CAPPluginCall) { perform(call) { try await $0.models(accountID: call.getString("accountId") ?? "") } }
    @objc func reconstruct(_ call: CAPPluginCall) {
        perform(call) { try await $0.reconstruct(accountID: call.getString("accountId") ?? "", model: call.getString("model") ?? "", image: call.getString("image") ?? "", instructions: call.getString("instructions") ?? "") }
    }
    @objc func manageUsage(_ call: CAPPluginCall) {
        DispatchQueue.main.async { UIApplication.shared.open(URL(string: "https://chatgpt.com/settings/usage")!); call.resolve() }
    }
}

private enum ChatGPTFailure: Error {
    case storage, connection, cancelled, authentication, identity, callback, plan, limit, incomplete, invalidInput, busy
    var message: String {
        switch self {
        case .storage: return "The secure ChatGPT account storage is unavailable. Unlock your device and try again."
        case .connection: return "The ChatGPT connection failed or timed out. Your scans are still here."
        case .cancelled: return "ChatGPT sign-in was cancelled."
        case .authentication: return "ChatGPT needs you to sign in again. Your scans are still here."
        case .identity: return "ChatGPT returned an identity that could not be verified. Please sign in again."
        case .callback: return "The ChatGPT sign-in callback could not be verified. Please try again."
        case .plan: return "ChatGPT plan access is not enabled for this account. Manage access in ChatGPT Settings or sign in again."
        case .limit: return "ChatGPT plan usage is currently unavailable or has reached a limit. Check ChatGPT Settings → Usage."
        case .incomplete: return "ChatGPT did not complete this page. Your scans are still here."
        case .invalidInput: return "Choose a ChatGPT account and an available image-capable model, then try again."
        case .busy: return "A ChatGPT operation is already running for this account."
        }
    }
}

private struct ChatGPTRegistration: Codable {
    var id: String
    var email: String = ""
    var label: String
    var subject: String = ""
    var accessToken: String?
    var refreshToken: String?
    var idToken: String?
    var expiresAt: Double = 0
    var earliestRefreshAt: Double = 0
    var scopes: [String] = []
    var generation: String = UUID().uuidString
    var connected: Bool { idToken != nil && (expiresAt > Date().timeIntervalSince1970 || refreshToken != nil) }
    var planEnabled: Bool { connected && accessToken != nil && scopes.contains("chatgpt.tokens.use.direct") }
    mutating func clear() { accessToken = nil; refreshToken = nil; idToken = nil; expiresAt = 0; earliestRefreshAt = 0; scopes = []; generation = UUID().uuidString }
}
private struct ChatGPTVault: Codable {
    var hostID = "urn:uuid:" + UUID().uuidString.lowercased()
    var accounts: [ChatGPTRegistration] = []
    var activeAccountID: String?
    static let service = "com.tandempdf.mobile.chatgpt.oauth"
    static func load() throws -> ChatGPTVault {
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service, kSecAttrAccount as String: "vault", kSecReturnData as String: true, kSecMatchLimit as String: kSecMatchLimitOne]
        var value: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &value)
        if status == errSecItemNotFound { let vault = ChatGPTVault(); try vault.save(); return vault }
        guard status == errSecSuccess, let data = value as? Data, let vault = try? JSONDecoder().decode(ChatGPTVault.self, from: data) else { throw ChatGPTFailure.storage }
        return vault
    }
    func save() throws {
        let data = try JSONEncoder().encode(self)
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: Self.service, kSecAttrAccount as String: "vault"]
        let update: [String: Any] = [kSecValueData as String: data, kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly]
        let status = SecItemUpdate(query as CFDictionary, update as CFDictionary)
        if status == errSecItemNotFound {
            guard SecItemAdd(query.merging(update) { _, new in new } as CFDictionary, nil) == errSecSuccess else { throw ChatGPTFailure.storage }
        } else if status != errSecSuccess { throw ChatGPTFailure.storage }
    }
}

@MainActor
private final class ChatGPTAccountService: NSObject, ASWebAuthenticationPresentationContextProviding {
    private var vault: ChatGPTVault
    weak var window: UIWindow?
    private var pending: OAuthAttempt?
    private var authSession: ASWebAuthenticationSession?
    private var refreshes: [String: Task<ChatGPTRegistration, Error>] = [:]
    private var requests: [String: Task<[String: Any], Error>] = [:]
    private var signingOut: Set<String> = []
    private var catalog: [String: Set<String>] = [:]
    init(vault: ChatGPTVault? = nil) throws { self.vault = try vault ?? ChatGPTVault.load() }
    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor { window ?? UIWindow() }
    func snapshot() -> [String: Any] {
        var result: [String: Any] = ["accounts": vault.accounts.map { ["id": $0.id, "email": $0.email, "label": $0.label, "connected": $0.connected, "planEnabled": $0.planEnabled] as [String: Any] }]
        if let id = vault.activeAccountID { result["activeAccountId"] = id }
        return result
    }
    private func account(_ id: String) throws -> ChatGPTRegistration {
        guard let value = vault.accounts.first(where: { $0.id == id }) else { throw ChatGPTFailure.authentication }
        return value
    }
    private func save(_ registration: ChatGPTRegistration, activate: Bool = false) throws {
        var next = vault
        if let index = next.accounts.firstIndex(where: { $0.id == registration.id }) { next.accounts[index] = registration }
        else { next.accounts.append(registration) }
        if activate { next.activeAccountID = registration.id }
        try next.save(); vault = next
    }
    func select(accountID: String) throws -> [String: Any] {
        guard try account(accountID).connected, !signingOut.contains(accountID) else { throw ChatGPTFailure.authentication }
        var next = vault; next.activeAccountID = accountID; try next.save(); vault = next
        return snapshot()
    }
    func signIn(accountID: String?) async throws -> [String: Any] {
        guard pending == nil else { throw ChatGPTFailure.busy }
        let saved = try accountID.map { try account($0) }
        if let id = accountID, signingOut.contains(id) || requests[id] != nil || refreshes[id] != nil { throw ChatGPTFailure.busy }
        let attempt = try OAuthAttempt(saved: saved)
        pending = attempt
        defer { if pending === attempt { pending = nil; authSession?.cancel(); authSession = nil; attempt.listener?.stop() } }
        let callback = try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<[String: String], Error>) in
            attempt.continuation = continuation
            do {
                let listener = try OAuthLoopback(state: attempt.state, finish: attempt.finishToken) { [weak self, weak attempt] result in
                    Task { @MainActor in
                        guard let self, let attempt, self.pending === attempt else { return }
                        attempt.finish(result)
                    }
                }
                attempt.listener = listener
                listener.start { [weak self, weak attempt] port in
                    Task { @MainActor in
                        guard let self, let attempt, self.pending === attempt else { return }
                        guard let port else { attempt.finish(.failure(ChatGPTFailure.connection)); return }
                        attempt.redirectURI = "http://127.0.0.1:\(port)/auth/callback"
                        var params = ["client_id": saved?.id ?? "dynamic_agent_client", "ext_agent_host_id": self.vault.hostID, "response_type": "code", "redirect_uri": attempt.redirectURI, "scope": "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct", "resource": "https://api.openai.com/v1", "state": attempt.state, "nonce": attempt.nonce, "code_challenge_method": "S256", "code_challenge": Data(SHA256.hash(data: Data(attempt.verifier.utf8))).base64URL]
                        if saved == nil { params["agent_name_hint"] = "Tandem PDF" }
                        if let saved, saved.connected, !saved.scopes.contains("chatgpt.tokens.use.direct") { params["prompt"] = "consent" }
                        if let token = saved?.idToken { params["id_token_hint"] = token }
                        if let email = saved?.email, !email.isEmpty { params["login_hint"] = email }
                        var url = URLComponents(string: "https://auth.openai.com/api/accounts/authorize")!
                        url.queryItems = params.map { URLQueryItem(name: $0.key, value: $0.value) }
                        let session = ASWebAuthenticationSession(url: url.url!, callbackURLScheme: "tandem-chatgpt-finish") { [weak self, weak attempt] url, _ in
                            Task { @MainActor in
                                guard let self, let attempt, self.pending === attempt else { return }
                                if let url, url.scheme == "tandem-chatgpt-finish", url.host == "complete", url.path == "/" + attempt.finishToken { return }
                                // Once the listener has accepted a callback, closing the browser is expected.
                                if attempt.continuation != nil { attempt.finish(.failure(ChatGPTFailure.cancelled)) }
                            }
                        }
                        session.presentationContextProvider = self
                        session.prefersEphemeralWebBrowserSession = true
                        self.authSession = session
                        if !session.start() { attempt.finish(.failure(ChatGPTFailure.connection)) }
                    }
                }
                Task { @MainActor [weak attempt] in
                    try? await Task.sleep(nanoseconds: 180_000_000_000)
                    attempt?.finish(.failure(ChatGPTFailure.cancelled))
                }
            } catch { attempt.finish(.failure(ChatGPTFailure.connection)) }
        }
        try Task.checkCancellation()
        guard pending === attempt else { throw ChatGPTFailure.cancelled }
        if callback["error"] != nil { throw ChatGPTFailure.authentication }
        guard let code = callback["code"], !code.isEmpty else { throw ChatGPTFailure.callback }
        let clientID: String
        if let saved {
            guard callback["client_id"] == nil || callback["client_id"] == saved.id else { throw ChatGPTFailure.identity }
            clientID = saved.id
        } else {
            guard let issued = callback["client_id"], issued != "dynamic_agent_client", issued.range(of: "^[A-Za-z0-9_-]{1,200}$", options: .regularExpression) != nil,
                  !vault.accounts.contains(where: { $0.id == issued }) else { throw ChatGPTFailure.callback }
            clientID = issued
            // Preserve the issued registration before exchanging a one-use code.
            try save(ChatGPTRegistration(id: issued, label: "ChatGPT account \(vault.accounts.count + 1)"))
        }
        let body = try await ChatGPTHTTP.json(url: "https://auth.openai.com/api/accounts/oauth/token", form: ["grant_type": "authorization_code", "client_id": clientID, "code": code, "code_verifier": attempt.verifier, "redirect_uri": attempt.redirectURI, "resource": "https://api.openai.com/v1"])
        let registration = try await credentials(body, previous: account(clientID), nonce: attempt.nonce, refreshing: false)
        guard pending === attempt else { throw ChatGPTFailure.cancelled }
        try save(registration, activate: true)
        catalog[clientID] = nil
        return snapshot()
    }
    func cancelSignIn() {
        let attempt = pending; pending = nil
        attempt?.finish(.failure(ChatGPTFailure.cancelled)); attempt?.listener?.stop()
        authSession?.cancel(); authSession = nil
    }
    private func credentials(_ body: [String: Any], previous: ChatGPTRegistration, nonce: String?, refreshing: Bool) async throws -> ChatGPTRegistration {
        let access = body["access_token"] as? String
        if let access {
            guard !access.isEmpty, access.count < 32_000, (body["token_type"] as? String)?.lowercased() == "bearer" else { throw ChatGPTFailure.authentication }
        }
        let expires = body["expires_in"] as? Double
        if let expires, expires <= 0 || expires > 86_400 { throw ChatGPTFailure.authentication }
        let refresh = body["refresh_token"] as? String
        if let refresh, refresh.isEmpty || refresh.count >= 32_000 { throw ChatGPTFailure.authentication }
        if refreshing && refresh == nil { throw ChatGPTFailure.authentication }
        let scope = body["scope"] as? String ?? (refreshing ? previous.scopes.joined(separator: " ") : "")
        guard scope.count <= 10_000 else { throw ChatGPTFailure.authentication }
        var next = previous
        next.scopes = scope.split(separator: " ").map(String.init)
        if let token = body["id_token"] as? String {
            let claims = try await ChatGPTJWT.verify(token, audience: previous.id, nonce: nonce)
            guard let subject = claims["sub"] as? String, !subject.isEmpty, previous.subject.isEmpty || subject == previous.subject else { throw ChatGPTFailure.identity }
            next.subject = subject
            next.expiresAt = claims["exp"] as? Double ?? 0
            if let email = claims["email"] as? String, email.count <= 320 { next.email = email }
            next.idToken = token
        } else if !refreshing { throw ChatGPTFailure.identity }
        // Identity-only grants are kept for the account picker but cannot invoke models.
        // Plan grants additionally bind the access token to this verified client and subject.
        if let expires { next.expiresAt = Date().timeIntervalSince1970 + expires }
        if next.scopes.contains("chatgpt.tokens.use.direct") {
            guard let access else { throw ChatGPTFailure.authentication }
            let accessClaims = try await ChatGPTJWT.verify(access, audience: "https://api.openai.com/v1", nonce: nil, authorizedParty: previous.id)
            guard accessClaims["sub"] as? String == next.subject, accessClaims["client_id"] as? String == previous.id else { throw ChatGPTFailure.identity }
            next.expiresAt = min(next.expiresAt, accessClaims["exp"] as? Double ?? 0)
        }
        next.accessToken = access; next.refreshToken = refresh
        if let timestamp = body["earliest_refresh_at"] as? Double { next.earliestRefreshAt = timestamp }
        else if let value = body["earliest_refresh_at"] as? String {
            let formatter = ISO8601DateFormatter(); formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
            let fractional = formatter.date(from: value)
            formatter.formatOptions = [.withInternetDateTime]
            guard let parsed = fractional?.timeIntervalSince1970 ?? formatter.date(from: value)?.timeIntervalSince1970 ?? Double(value), parsed.isFinite, parsed >= 0 else { throw ChatGPTFailure.authentication }
            next.earliestRefreshAt = parsed
        } else if body["earliest_refresh_at"] == nil || body["earliest_refresh_at"] is NSNull { next.earliestRefreshAt = 0 }
        else { throw ChatGPTFailure.authentication }
        guard next.earliestRefreshAt.isFinite, next.earliestRefreshAt >= 0 else { throw ChatGPTFailure.authentication }
        return next
    }
    private func authorized(_ id: String) async throws -> ChatGPTRegistration {
        guard !signingOut.contains(id) else { throw ChatGPTFailure.authentication }
        let saved = try account(id)
        guard saved.planEnabled else { throw ChatGPTFailure.plan }
        if saved.expiresAt > Date().timeIntervalSince1970 + 90 { return saved }
        if let refresh = refreshes[id] { return try await refresh.value }
        guard let token = saved.refreshToken else { throw ChatGPTFailure.authentication }
        if saved.earliestRefreshAt > Date().timeIntervalSince1970 {
            guard saved.expiresAt > Date().timeIntervalSince1970 else { throw ChatGPTFailure.authentication }
            return saved
        }
        let task = Task { @MainActor in
            let body = try await ChatGPTHTTP.json(url: "https://auth.openai.com/api/accounts/oauth/token", form: ["grant_type": "refresh_token", "client_id": saved.id, "refresh_token": token, "resource": "https://api.openai.com/v1"])
            let renewed = try await self.credentials(body, previous: saved, nonce: nil, refreshing: true)
            try Task.checkCancellation()
            guard !self.signingOut.contains(id), try self.account(id).generation == saved.generation else { throw ChatGPTFailure.authentication }
            try self.save(renewed)
            guard renewed.planEnabled else { throw ChatGPTFailure.plan }
            return renewed
        }
        refreshes[id] = task
        defer { refreshes[id] = nil }
        do { return try await task.value }
        catch ChatGPTFailure.authentication {
            if var current = try? account(id), current.generation == saved.generation, !signingOut.contains(id) {
                current.clear(); try save(current); catalog[id] = nil
            }
            throw ChatGPTFailure.authentication
        }
    }
    func signOut(accountID: String) async throws -> [String: Any] {
        var saved = try account(accountID)
        guard !signingOut.contains(accountID) else { throw ChatGPTFailure.busy }
        signingOut.insert(accountID)
        defer { signingOut.remove(accountID) }
        if pending?.saved?.id == accountID { cancelSignIn() }
        requests[accountID]?.cancel(); requests[accountID] = nil
        refreshes[accountID]?.cancel(); refreshes[accountID] = nil
        catalog[accountID] = nil
        let refresh = saved.refreshToken
        var revoked = refresh == nil
        if let refresh {
            do {
                let discovery = try await ChatGPTHTTP.json(url: "https://auth.openai.com/.well-known/openid-configuration")
                guard discovery["issuer"] as? String == "https://auth.openai.com", let endpoint = discovery["revocation_endpoint"] as? String,
                      let url = URLComponents(string: endpoint), url.scheme == "https", url.host == "auth.openai.com", url.user == nil, url.password == nil, url.port == nil, url.query == nil, url.fragment == nil else { throw ChatGPTFailure.authentication }
                for attempt in 0..<2 {
                    do {
                        let response = try await ChatGPTHTTP.send(url: endpoint, form: ["token": refresh, "token_type_hint": "refresh_token", "client_id": accountID])
                        revoked = response.0.statusCode == 200
                        if revoked || response.0.statusCode < 500 { break }
                    } catch { if attempt == 1 { break } }
                    if attempt == 0 { try await Task.sleep(nanoseconds: 1_000_000_000) }
                }
            } catch { revoked = false }
        }
        saved.clear(); try save(saved)
        if vault.activeAccountID == accountID { var next = vault; next.activeAccountID = nil; try next.save(); vault = next }
        var result = snapshot(); result["revoked"] = revoked; return result
    }
    func models(accountID: String) async throws -> [String: Any] {
        let saved = try await authorized(accountID)
        let body = try await ChatGPTHTTP.json(url: "https://api.openai.com/v1/models", bearer: saved.accessToken)
        guard try account(accountID).generation == saved.generation, !signingOut.contains(accountID), let list = body["models"] as? [[String: Any]] else { throw ChatGPTFailure.authentication }
        let models = list.compactMap { entry -> [String: String]? in
            guard entry["visibility"] as? String == "list", let id = entry["slug"] as? String, id.range(of: "^[A-Za-z0-9._:-]{1,120}$", options: .regularExpression) != nil,
                  let name = entry["display_name"] as? String, name.count <= 200 else { return nil }
            return ["id": id, "name": name]
        }
        catalog[accountID] = Set(models.compactMap { $0["id"] })
        return ["models": models]
    }
    func reconstruct(accountID: String, model: String, image: String, instructions: String) async throws -> [String: Any] {
        guard requests[accountID] == nil, !signingOut.contains(accountID) else { throw ChatGPTFailure.busy }
        guard image.utf8.count <= 12_000_000, image.hasPrefix("data:image/jpeg;base64,"), let bytes = Data(base64Encoded: String(image.dropFirst(23))), bytes.count > 2, bytes[0] == 0xff, bytes[1] == 0xd8,
              !instructions.isEmpty, instructions.utf8.count <= 16_000 else { throw ChatGPTFailure.invalidInput }
        let task: Task<[String: Any], Error> = Task { @MainActor in
            let saved = try await self.authorized(accountID)
            if self.catalog[accountID] == nil { _ = try await self.models(accountID: accountID) }
            guard self.catalog[accountID]?.contains(model) == true else { throw ChatGPTFailure.invalidInput }
            try Task.checkCancellation()
            let body: [String: Any] = ["model": model, "instructions": instructions, "store": false, "stream": true, "input": [["role": "user", "content": [["type": "input_image", "image_url": image, "detail": "high"], ["type": "input_text", "text": "Transcribe this page."]]]]]
            let response = try await ChatGPTHTTP.send(url: "https://api.openai.com/v1/responses", json: body, bearer: saved.accessToken, streaming: true)
            try Task.checkCancellation()
            guard try self.account(accountID).generation == saved.generation, !self.signingOut.contains(accountID) else { throw ChatGPTFailure.authentication }
            try ChatGPTHTTP.checkStatus(response.0.statusCode)
            guard response.0.mimeType?.lowercased() == "text/event-stream" else { throw ChatGPTFailure.incomplete }
            return ["response": try ChatGPTStream.completed(response.1)]
        }
        requests[accountID] = task
        defer { requests[accountID] = nil }
        return try await task.value
    }
}

@MainActor
private final class OAuthAttempt {
    let saved: ChatGPTRegistration?
    let state: String, nonce: String, verifier: String, finishToken: String
    var redirectURI = ""
    var listener: OAuthLoopback?
    var continuation: CheckedContinuation<[String: String], Error>?
    init(saved: ChatGPTRegistration?) throws {
        self.saved = saved; state = try Self.random(); nonce = try Self.random(); verifier = try Self.random(); finishToken = try Self.random()
    }
    static func random() throws -> String {
        var data = Data(count: 32)
        guard data.withUnsafeMutableBytes({ SecRandomCopyBytes(kSecRandomDefault, 32, $0.baseAddress!) }) == errSecSuccess else { throw ChatGPTFailure.authentication }
        return data.base64URL
    }
    func finish(_ result: Result<[String: String], Error>) { let completion = continuation; continuation = nil; completion?.resume(with: result) }
}

private final class OAuthLoopback {
    private let listener: NWListener
    private let queue = DispatchQueue(label: "com.tandempdf.oauth.loopback")
    private let state: String, finish: String
    private let callback: (Result<[String: String], Error>) -> Void
    private var connections: [NWConnection] = []
    private var consumed = false
    init(state: String, finish: String, callback: @escaping (Result<[String: String], Error>) -> Void) throws {
        self.state = state; self.finish = finish; self.callback = callback
        let parameters = NWParameters.tcp
        parameters.requiredLocalEndpoint = .hostPort(host: "127.0.0.1", port: .any)
        self.listener = try NWListener(using: parameters)
    }
    func start(ready: @escaping (UInt16?) -> Void) {
        var readySent = false
        listener.stateUpdateHandler = { [weak self] value in
            guard let self else { return }
            switch value {
            case .ready: if !readySent { readySent = true; ready(self.listener.port?.rawValue) }
            case .failed: if !readySent { readySent = true; ready(nil) }; self.stop()
            default: break
            }
        }
        listener.newConnectionHandler = { [weak self] connection in
            guard let self, self.connections.count < 8 else { connection.cancel(); return }
            self.connections.append(connection); connection.start(queue: self.queue)
            self.receive(connection, data: Data())
            self.queue.asyncAfter(deadline: .now() + 10) { connection.cancel(); self.connections.removeAll { $0 === connection } }
        }
        listener.start(queue: queue)
    }
    func stop() { queue.async { self.listener.cancel(); self.connections.forEach { $0.cancel() }; self.connections.removeAll() } }
    private func receive(_ connection: NWConnection, data: Data) {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 16_384) { [weak self] part, _, complete, error in
            guard let self else { connection.cancel(); return }
            var data = data; if let part { data.append(part) }
            guard data.count <= 16_384, error == nil else { connection.cancel(); return }
            if let request = String(data: data, encoding: .utf8), request.contains("\r\n\r\n") {
                self.handle(connection, request: request)
            } else if !complete { self.receive(connection, data: data) } else { connection.cancel() }
        }
    }
    private func handle(_ connection: NWConnection, request: String) {
        guard !consumed, let port = listener.port?.rawValue,
              let params = ChatGPTOAuthCallback.parse(request, state: state, port: port) else {
            respond(connection, status: "400 Bad Request", body: "Invalid callback."); return
        }
        consumed = true
        // Only a local completion nonce reaches the custom scheme; OAuth credentials stay here.
        respond(connection, status: "302 Found", body: "Return to Tandem PDF.", extra: "Location: tandem-chatgpt-finish://complete/\(finish)\r\n")
        callback(.success(params))
    }
    private func respond(_ connection: NWConnection, status: String, body: String, extra: String = "") {
        let response = "HTTP/1.1 \(status)\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nPragma: no-cache\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\(extra)Content-Length: \(body.utf8.count)\r\n\r\n\(body)"
        connection.send(content: Data(response.utf8), completion: .contentProcessed { _ in connection.cancel() })
    }
}

private enum ChatGPTOAuthCallback {
    static func parse(_ request: String, state: String, port: UInt16) -> [String: String]? {
        guard request.utf8.count <= 16_384 else { return nil }
        let lines = request.components(separatedBy: "\r\n")
        let first = (lines.first ?? "").split(separator: " ")
        let hosts = lines.filter { $0.lowercased().hasPrefix("host:") }
        guard first.count == 3, first[0] == "GET", first[2] == "HTTP/1.1", first[1].hasPrefix("/auth/callback?"),
              hosts.count == 1, hosts[0].lowercased() == "host: 127.0.0.1:\(port)",
              let components = URLComponents(string: "http://127.0.0.1:\(port)" + first[1]), components.path == "/auth/callback", components.fragment == nil else { return nil }
        var params: [String: String] = [:]
        for item in components.queryItems ?? [] {
            guard params[item.name] == nil, let value = item.value else { return nil }
            params[item.name] = value
        }
        guard params["state"] == state else { return nil }
        return params
    }
}

private final class ChatGPTHTTP: NSObject, URLSessionDataDelegate, URLSessionTaskDelegate {
    private var continuation: CheckedContinuation<(HTTPURLResponse, Data), Error>?
    private var data = Data()
    private var response: HTTPURLResponse?
    private var session: URLSession?
    private let cancellationLock = NSLock()
    private var cancelled = false
    private let limit: Int
    private func cancel() {
        cancellationLock.lock(); cancelled = true; let current = session; cancellationLock.unlock()
        current?.invalidateAndCancel()
    }
    private func start(_ session: URLSession, request: URLRequest) {
        cancellationLock.lock()
        self.session = session
        if cancelled {
            cancellationLock.unlock()
            let completion = continuation; continuation = nil
            session.invalidateAndCancel(); completion?.resume(throwing: CancellationError())
        } else {
            session.dataTask(with: request).resume()
            cancellationLock.unlock()
        }
    }
    private init(limit: Int) { self.limit = limit }
    static func checkStatus(_ status: Int) throws {
        if status == 401 || status == 403 { throw ChatGPTFailure.authentication }
        if status == 429 { throw ChatGPTFailure.limit }
        guard (200..<300).contains(status) else { throw ChatGPTFailure.connection }
    }
    static func json(url: String, form: [String: String]? = nil, bearer: String? = nil) async throws -> [String: Any] {
        let response = try await send(url: url, form: form, bearer: bearer)
        if url == "https://auth.openai.com/api/accounts/oauth/token", response.0.statusCode == 400,
           let errorBody = try? JSONSerialization.jsonObject(with: response.1) as? [String: Any], errorBody["error"] as? String == "invalid_grant" { throw ChatGPTFailure.authentication }
        try checkStatus(response.0.statusCode)
        guard let body = try? JSONSerialization.jsonObject(with: response.1) as? [String: Any] else { throw ChatGPTFailure.connection }
        return body
    }
    static func send(url: String, form: [String: String]? = nil, json: [String: Any]? = nil, bearer: String? = nil, streaming: Bool = false) async throws -> (HTTPURLResponse, Data) {
        guard let url = URL(string: url), url.scheme == "https", ["auth.openai.com", "api.openai.com"].contains(url.host ?? ""), url.user == nil, url.password == nil else { throw ChatGPTFailure.connection }
        var request = URLRequest(url: url)
        request.timeoutInterval = streaming ? 120 : 30
        if let form {
            var components = URLComponents(); components.queryItems = form.map { URLQueryItem(name: $0.key, value: $0.value) }
            request.httpBody = components.percentEncodedQuery?.replacingOccurrences(of: "+", with: "%2B").data(using: .utf8)
            request.httpMethod = "POST"; request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        }
        if let json { request.httpBody = try JSONSerialization.data(withJSONObject: json); request.httpMethod = "POST"; request.setValue("application/json", forHTTPHeaderField: "Content-Type") }
        if let bearer { request.setValue("Bearer " + bearer, forHTTPHeaderField: "Authorization") }
        request.setValue(streaming ? "text/event-stream" : "application/json", forHTTPHeaderField: "Accept")
        let transport = ChatGPTHTTP(limit: streaming ? 8_000_000 : 1_000_000)
        return try await withTaskCancellationHandler(operation: {
            try Task.checkCancellation()
            return try await withCheckedThrowingContinuation { continuation in
                transport.continuation = continuation
                let config = URLSessionConfiguration.ephemeral
                config.httpShouldSetCookies = false; config.httpCookieStorage = nil; config.urlCache = nil
                config.timeoutIntervalForRequest = streaming ? 120 : 30; config.timeoutIntervalForResource = streaming ? 180 : 45
                let session = URLSession(configuration: config, delegate: transport, delegateQueue: nil)
                transport.start(session, request: request)
            }
        }, onCancel: { transport.cancel() })
    }
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) { completionHandler(nil) }
    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse, completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        guard let response = response as? HTTPURLResponse, response.expectedContentLength <= Int64(limit) else { completionHandler(.cancel); return }
        self.response = response; completionHandler(.allow)
    }
    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        guard self.data.count + data.count <= limit else { dataTask.cancel(); return }
        self.data.append(data)
    }
    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        let completion = continuation; continuation = nil
        defer { session.invalidateAndCancel(); cancellationLock.lock(); self.session = nil; cancellationLock.unlock() }
        guard error == nil, let response else { completion?.resume(throwing: ChatGPTFailure.connection); return }
        completion?.resume(returning: (response, data))
    }
}

private enum ChatGPTJWT {
    static func verify(_ token: String, audience: String, nonce: String?, authorizedParty: String? = nil) async throws -> [String: Any] {
        let jwks = try await ChatGPTHTTP.json(url: "https://auth.openai.com/.well-known/jwks.json")
        return try verify(token, audience: audience, nonce: nonce, authorizedParty: authorizedParty, jwks: jwks)
    }
    static func verify(_ token: String, audience: String, nonce: String?, authorizedParty: String? = nil, jwks: [String: Any]) throws -> [String: Any] {
        guard token.utf8.count < 32_000 else { throw ChatGPTFailure.identity }
        let parts = token.split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count == 3, let headerData = Data(base64URL: String(parts[0])), let claimsData = Data(base64URL: String(parts[1])), let signature = Data(base64URL: String(parts[2])),
              let header = try? JSONSerialization.jsonObject(with: headerData) as? [String: Any], header["alg"] as? String == "RS256", header["crit"] == nil,
              let kid = header["kid"] as? String, let claims = try? JSONSerialization.jsonObject(with: claimsData) as? [String: Any] else { throw ChatGPTFailure.identity }
        guard let keys = jwks["keys"] as? [[String: Any]] else { throw ChatGPTFailure.identity }
        let matches = keys.filter { $0["kid"] as? String == kid && $0["kty"] as? String == "RSA" && ($0["use"] as? String == "sig" || $0["use"] == nil) && ($0["alg"] as? String == "RS256" || $0["alg"] == nil) }
        guard matches.count == 1, let n = matches[0]["n"] as? String, let e = matches[0]["e"] as? String, let modulus = Data(base64URL: n), modulus.count >= 256, let exponent = Data(base64URL: e) else { throw ChatGPTFailure.identity }
        let keyData = der(0x30, derInteger(modulus) + derInteger(exponent))
        guard let key = SecKeyCreateWithData(keyData as CFData, [kSecAttrKeyType: kSecAttrKeyTypeRSA, kSecAttrKeyClass: kSecAttrKeyClassPublic] as CFDictionary, nil),
              SecKeyVerifySignature(key, .rsaSignatureMessagePKCS1v15SHA256, Data((parts[0] + "." + parts[1]).utf8) as CFData, signature as CFData, nil) else { throw ChatGPTFailure.identity }
        let now = Date().timeIntervalSince1970
        let audiences = (claims["aud"] as? [String]) ?? (claims["aud"] as? String).map { [$0] } ?? []
        guard claims["iss"] as? String == "https://auth.openai.com", audiences.contains(audience), let exp = claims["exp"] as? Double, exp > now - 30,
              let subject = claims["sub"] as? String, !subject.isEmpty, subject.count < 1000 else { throw ChatGPTFailure.identity }
        if (audiences.count > 1 || claims["azp"] != nil), claims["azp"] as? String != (authorizedParty ?? audience) { throw ChatGPTFailure.identity }
        if let nbf = claims["nbf"] as? Double, nbf > now + 30 { throw ChatGPTFailure.identity }
        if let nonce, claims["nonce"] as? String != nonce { throw ChatGPTFailure.identity }
        return claims
    }
    private static func derInteger(_ input: Data) -> Data { var bytes = input; while bytes.count > 1 && bytes.first == 0 { bytes.removeFirst() }; if let first = bytes.first, first & 0x80 != 0 { bytes.insert(0, at: 0) }; return der(0x02, bytes) }
    private static func der(_ tag: UInt8, _ data: Data) -> Data {
        var result = Data([tag])
        if data.count < 128 { result.append(UInt8(data.count)) }
        else { var size = data.count; var bytes: [UInt8] = []; while size > 0 { bytes.insert(UInt8(size & 255), at: 0); size >>= 8 }; result.append(UInt8(0x80 | bytes.count)); result.append(contentsOf: bytes) }
        result.append(data); return result
    }
}

private enum ChatGPTStream {
    static func completed(_ data: Data) throws -> [String: Any] {
        guard let source = String(data: data, encoding: .utf8) else { throw ChatGPTFailure.incomplete }
        var final: [String: Any]?
        for block in source.replacingOccurrences(of: "\r\n", with: "\n").replacingOccurrences(of: "\r", with: "\n").components(separatedBy: "\n\n").dropLast() {
            let content = block.components(separatedBy: "\n").filter { $0.hasPrefix("data:") }.map { String($0.dropFirst(5)).trimmingCharacters(in: .whitespaces) }.joined(separator: "\n")
            if content.isEmpty || content == "[DONE]" { continue }
            guard let bytes = content.data(using: .utf8), let event = try? JSONSerialization.jsonObject(with: bytes) as? [String: Any], let type = event["type"] as? String else { throw ChatGPTFailure.incomplete }
            if type == "error" || type == "response.failed" || type == "response.incomplete" {
                let response = event["response"] as? [String: Any]
                let error = (response?["error"] as? [String: Any]) ?? (event["error"] as? [String: Any]) ?? event
                if let code = error["code"] as? String, ["subscription_sharing_usage_limit_exceeded", "subscription_sharing_usage_unavailable"].contains(code) { throw ChatGPTFailure.limit }
                throw ChatGPTFailure.incomplete
            }
            if type == "response.completed" {
                guard final == nil, let response = event["response"] as? [String: Any], response["status"] as? String == "completed", response["error"] == nil || response["error"] is NSNull,
                      response["incomplete_details"] == nil || response["incomplete_details"] is NSNull,
                      let output = response["output"] as? [[String: Any]] else { throw ChatGPTFailure.incomplete }
                var texts: [String] = []
                for item in output {
                    if item["type"] as? String == "reasoning" { continue }
                    guard item["type"] as? String == "message", item["status"] == nil || item["status"] as? String == "completed", let content = item["content"] as? [[String: Any]] else { throw ChatGPTFailure.incomplete }
                    for part in content {
                        guard part["type"] as? String == "output_text", let text = part["text"] as? String else { throw ChatGPTFailure.incomplete }
                        texts.append(text)
                    }
                }
                let text = texts.joined(separator: "\n")
                guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, text.utf16.count <= 24_000 else { throw ChatGPTFailure.incomplete }
                // Whitelist the bridge payload; provider metadata can contain opaque secrets.
                final = ["status": "completed", "output": [["type": "message", "status": "completed", "content": [["type": "output_text", "text": text]]]]]
            }
        }
        guard let final else { throw ChatGPTFailure.incomplete }
        return final
    }
}

private extension Data {
    var base64URL: String { base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "") }
    init?(base64URL: String) {
        guard !base64URL.isEmpty, base64URL.range(of: "^[A-Za-z0-9_-]+$", options: .regularExpression) != nil else { return nil }
        let value = base64URL.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        self.init(base64Encoded: value + String(repeating: "=", count: (4 - value.count % 4) % 4))
    }
}
