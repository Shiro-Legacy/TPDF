package com.tandempdf.mobile;

import android.content.Intent;
import android.net.Uri;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.AtomicFile;
import com.getcapacitor.JSArray;
import com.getcapacitor.JSObject;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginCall;
import com.getcapacitor.PluginMethod;
import com.getcapacitor.annotation.CapacitorPlugin;
import org.json.JSONArray;
import org.json.JSONObject;
import java.io.BufferedInputStream;
import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.math.BigInteger;
import java.net.HttpURLConnection;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.net.SocketTimeoutException;
import java.net.URI;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.security.KeyFactory;
import java.security.KeyStore;
import java.security.Signature;
import java.security.spec.RSAPublicKeySpec;
import java.util.Arrays;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;

/** OAuth credentials remain native. The bridge returns only account summaries and finished documents. */
@CapacitorPlugin(name = "ChatGPTAccount")
public class ChatGPTAccountPlugin extends Plugin {
    private static final String ISSUER = "https://auth.openai.com";
    private static final String AUTHORIZE = ISSUER + "/api/accounts/authorize";
    private static final String TOKEN = ISSUER + "/api/accounts/oauth/token";
    private static final String REVOKE = ISSUER + "/api/accounts/oauth/revoke";
    private static final String DISCOVERY = ISSUER + "/.well-known/openid-configuration";
    private static final String JWKS = ISSUER + "/.well-known/jwks.json";
    private static final String API = "https://api.openai.com/v1";
    private static final String DIRECT = "chatgpt.tokens.use.direct";
    private static final String KEY_ALIAS = "com.tandempdf.mobile.chatgpt.v1";
    private static final byte[] AAD = "Tandem ChatGPT accounts v1".getBytes(StandardCharsets.US_ASCII);
    private final ExecutorService serial = Executors.newSingleThreadExecutor();
    private final ExecutorService listener = Executors.newSingleThreadExecutor();
    private final ScheduledExecutorService deadlines = Executors.newSingleThreadScheduledExecutor();
    private final Map<String, HttpURLConnection> requests = new ConcurrentHashMap<>();
    private final Set<HttpURLConnection> allRequests = ConcurrentHashMap.newKeySet();
    private final Map<HttpURLConnection, ScheduledFuture<?>> requestDeadlines = new ConcurrentHashMap<>();
    private final Object requestGate = new Object();
    private final Set<String> stopped = ConcurrentHashMap.newKeySet();
    private final Map<String, Set<String>> catalog = new java.util.HashMap<>();
    private volatile Attempt pending;
    private volatile boolean destroyed;
    private JSONObject cachedKeys;
    private long keysAt;

    private static final class SafeFailure extends Exception {
        SafeFailure(String message) { super(message); }
    }
    private static final class HttpFailure extends IOException {
        final int status;
        final String code;
        HttpFailure(int status, String code) { this.status = status; this.code = code; }
    }
    private static final class Attempt {
        final PluginCall call;
        final ServerSocket socket;
        final String requestedId, state, nonce, verifier, redirect;
        final AtomicBoolean completed = new AtomicBoolean();
        final long expires = System.currentTimeMillis() + 10 * 60_000;
        Attempt(PluginCall call, ServerSocket socket, String account) throws Exception {
            this.call = call; this.socket = socket; requestedId = account;
            state = ChatGPTProtocol.random(32); nonce = ChatGPTProtocol.random(32); verifier = ChatGPTProtocol.random(64);
            redirect = "http://127.0.0.1:" + socket.getLocalPort() + "/auth/callback";
        }
    }
    private interface Action { void run() throws Exception; }
    private void perform(PluginCall call, Action action) {
        if (destroyed) { call.reject("ChatGPT connection is closed. Reopen Tandem PDF."); return; }
        serial.execute(() -> {
            try { action.run(); }
            catch (SafeFailure error) { call.reject(error.getMessage()); }
            catch (Exception error) { call.reject("The ChatGPT operation could not finish. Your scans are still here. Try again or reconnect the account."); }
        });
    }
    @PluginMethod public void getAccounts(PluginCall call) {
        perform(call, () -> call.resolve(snapshot(loadStore())));
    }
    @PluginMethod public void selectAccount(PluginCall call) {
        perform(call, () -> {
            JSONObject store = loadStore(), account = account(store, call.getString("accountId"));
            if (!connected(account)) throw new SafeFailure("Sign in to this ChatGPT account first.");
            store.put("activeAccountId", account.getString("id")); saveStore(store); call.resolve(snapshot(store));
        });
    }
    @PluginMethod public void signIn(PluginCall call) {
        perform(call, () -> {
            if (pending != null) throw new SafeFailure("A ChatGPT sign-in is already open.");
            JSONObject store = loadStore(); String requested = call.getString("accountId");
            if (requested == null && store.getJSONArray("accounts").length() >= 50) throw new SafeFailure("This installation already has 50 ChatGPT registrations. Choose a saved account.");
            JSONObject old = requested == null ? null : account(store, requested);
            ServerSocket server = new ServerSocket();
            server.bind(new InetSocketAddress(InetAddress.getByName("127.0.0.1"), 0), 4); server.setSoTimeout(1000);
            Attempt attempt = new Attempt(call, server, requested); pending = attempt;
            Map<String, String> values = new LinkedHashMap<>();
            values.put("client_id", requested == null ? "dynamic_agent_client" : requested);
            if (requested == null) values.put("agent_name_hint", "Tandem PDF");
            else {
                if (!old.optString("id_token").isEmpty()) values.put("id_token_hint", old.getString("id_token"));
                if (!old.optString("email").isEmpty()) values.put("login_hint", old.getString("email"));
                if (connected(old) && !hasScope(old, DIRECT)) values.put("prompt", "consent");
            }
            values.put("ext_agent_host_id", store.getString("host"));
            values.put("response_type", "code"); values.put("redirect_uri", attempt.redirect);
            values.put("scope", "openid profile email offline_access resource.invoke " + DIRECT);
            values.put("resource", API); values.put("state", attempt.state); values.put("nonce", attempt.nonce);
            values.put("code_challenge_method", "S256"); values.put("code_challenge", ChatGPTProtocol.challenge(attempt.verifier));
            String authorization = AUTHORIZE + "?" + ChatGPTProtocol.form(values);
            listener.execute(() -> listen(attempt));
            getActivity().runOnUiThread(() -> {
                if (!live(attempt)) return;
                try { getActivity().startActivity(new Intent(Intent.ACTION_VIEW, Uri.parse(authorization)).addCategory(Intent.CATEGORY_BROWSABLE)); }
                catch (Exception error) { finish(attempt, null, "A system browser is required to sign in to ChatGPT."); }
            });
        });
    }
    @PluginMethod public void cancelSignIn(PluginCall call) {
        Attempt attempt = pending;
        if (attempt != null) finish(attempt, null, "ChatGPT sign-in was cancelled.");
        call.resolve();
    }
    private boolean live(Attempt attempt) { return pending == attempt && !attempt.completed.get() && !destroyed && System.currentTimeMillis() < attempt.expires; }
    private void finish(Attempt attempt, JSObject value, String error) {
        if (!attempt.completed.compareAndSet(false, true)) return;
        try { attempt.socket.close(); } catch (Exception ignored) {}
        if (pending == attempt) pending = null;
        if (error != null) attempt.call.reject(error); else attempt.call.resolve(value);
    }
    private void listen(Attempt attempt) {
        try {
            while (live(attempt)) {
                try (Socket peer = attempt.socket.accept()) {
                    peer.setSoTimeout(3000);
                    if (!peer.getInetAddress().isLoopbackAddress()) continue;
                    InputStream input = new BufferedInputStream(peer.getInputStream());
                    String first = ChatGPTProtocol.line(input, 16384);
                    if (first == null) continue;
                    String[] request = first.split(" ");
                    int length = 0; String host = null, line;
                    while ((line = ChatGPTProtocol.line(input, 4096)) != null && !line.isEmpty()) {
                        length += line.length(); if (length > 16384) throw new IOException();
                        if (line.toLowerCase(java.util.Locale.ROOT).startsWith("host:")) host = line.substring(5).trim();
                    }
                    if (request.length != 3 || !"GET".equals(request[0]) || !request[2].startsWith("HTTP/1.") ||
                        !("127.0.0.1:" + attempt.socket.getLocalPort()).equals(host)) { page(peer, false); continue; }
                    URI uri = new URI(request[1]);
                    if (uri.isAbsolute() || uri.getRawFragment() != null || !"/auth/callback".equals(uri.getRawPath())) { page(peer, false); continue; }
                    Map<String, String> fields = ChatGPTProtocol.query(uri.getRawQuery());
                    if (!ChatGPTProtocol.equal(attempt.state, fields.get("state"))) {
                        page(peer, false); finish(attempt, null, "ChatGPT sign-in could not be verified. Start again."); return;
                    }
                    page(peer, true); attempt.socket.close();
                    serial.execute(() -> exchange(attempt, fields)); return;
                } catch (SocketTimeoutException ignored) { /* Wait only until this attempt's fixed deadline. */ }
            }
            if (!attempt.completed.get()) finish(attempt, null, "ChatGPT sign-in timed out. Start again.");
        } catch (Exception error) {
            if (!attempt.completed.get()) finish(attempt, null, "ChatGPT sign-in could not be completed. Start again.");
        }
    }
    private void page(Socket peer, boolean accepted) throws IOException {
        String message = accepted ? "Return to Tandem PDF to finish connecting your ChatGPT account." : "This sign-in request could not be verified. Return to Tandem PDF and try again.";
        byte[] body = ("<!doctype html><html><head><meta name=\"viewport\" content=\"width=device-width\"><title>Tandem PDF</title></head><body><h1>Tandem PDF</h1><p>" + message +
            "</p><p><a href=\"tandempdf://auth/return\">Return to Tandem PDF</a></p></body></html>").getBytes(StandardCharsets.UTF_8);
        String headers = "HTTP/1.1 " + (accepted ? "200 OK" : "400 Bad Request") + "\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; base-uri 'none'; frame-ancestors 'none'\r\nConnection: close\r\nContent-Length: " + body.length + "\r\n\r\n";
        peer.getOutputStream().write(headers.getBytes(StandardCharsets.US_ASCII)); peer.getOutputStream().write(body); peer.getOutputStream().flush();
    }
    private void exchange(Attempt attempt, Map<String, String> fields) {
        try {
            if (!live(attempt)) return;
            if (fields.containsKey("error")) throw new SafeFailure("ChatGPT sign-in was not authorized. You can try again or use an API key.");
            String id = fields.get("client_id");
            if (attempt.requestedId != null) {
                if (id != null && !id.equals(attempt.requestedId)) throw new SafeFailure("ChatGPT returned a different registration. Sign-in was stopped.");
                id = attempt.requestedId;
            }
            if (id == null || !id.matches("[A-Za-z0-9_-]{1,200}") || "dynamic_agent_client".equals(id)) throw new SafeFailure("ChatGPT registration was incomplete. Start again.");
            String code = fields.get("code");
            if (code == null || code.isEmpty() || code.length() > 8192) throw new SafeFailure("ChatGPT sign-in was incomplete. Start again.");
            JSONObject store = loadStore(); JSONObject record = findAccount(store, id);
            if (attempt.requestedId == null && record != null) throw new SafeFailure("This registration already exists. Choose its saved account and sign in again.");
            if (record == null) {
                record = new JSONObject().put("id", id).put("label", "ChatGPT account " + (store.getJSONArray("accounts").length() + 1));
                store.getJSONArray("accounts").put(record);
                // Preserve the issued registration even if exchanging this one-use code fails.
                saveStore(store);
            }
            JSONObject tokens = jsonRequest(TOKEN, "POST", "application/x-www-form-urlencoded", null,
                ChatGPTProtocol.form(fields("grant_type", "authorization_code", "client_id", id, "code", code,
                    "code_verifier", attempt.verifier, "redirect_uri", attempt.redirect, "resource", API)), null);
            JSONObject identity = identity(tokens.getString("id_token"), id, attempt.nonce);
            if (!record.optString("subject").isEmpty() && !record.getString("subject").equals(identity.getString("sub"))) throw new SafeFailure("ChatGPT returned a different account. The saved connection was not replaced.");
            if (!live(attempt)) return;
            applyTokens(record, tokens, identity, false); store.put("activeAccountId", id); saveStore(store);
            stopped.remove(id); catalog.remove(id); finish(attempt, snapshot(store), null);
        } catch (SafeFailure error) { finish(attempt, null, error.getMessage()); }
        catch (Exception error) { finish(attempt, null, "ChatGPT sign-in could not be verified. Select the saved account and sign in again."); }
    }
    @PluginMethod public void signOut(PluginCall call) {
        String id = call.getString("accountId");
        if (id != null) {
            synchronized (requestGate) {
                stopped.add(id); HttpURLConnection request = requests.get(id); if (request != null) request.disconnect();
            }
            Attempt attempt = pending; if (attempt != null && id.equals(attempt.requestedId)) finish(attempt, null, "ChatGPT sign-in was cancelled.");
        }
        perform(call, () -> {
            JSONObject store = loadStore(), record = account(store, id);
            String renewable = record.optString("refresh_token"); boolean revoked = renewable.isEmpty();
            clearTokens(record);
            if (id.equals(store.optString("activeAccountId"))) store.remove("activeAccountId");
            saveStore(store); catalog.remove(id);
            if (!revoked) {
                try {
                    JSONObject discovery = jsonRequest(DISCOVERY, "GET", null, null, null, null);
                    if (!ISSUER.equals(discovery.optString("issuer")) || !REVOKE.equals(discovery.optString("revocation_endpoint"))) throw new IOException();
                    String body = ChatGPTProtocol.form(fields("token", renewable, "token_type_hint", "refresh_token", "client_id", id));
                    for (int i = 0; i < 2; i++) {
                        try { requestBytes(REVOKE, "POST", "application/x-www-form-urlencoded", null, body, null); revoked = true; break; }
                        catch (HttpFailure error) { if (error.status < 500 || i == 1) break; Thread.sleep(400); }
                        catch (IOException error) { if (i == 1) break; Thread.sleep(400); }
                    }
                } catch (Exception ignored) { /* Local sign-out still removes every credential. */ }
            }
            call.resolve(snapshot(store).put("revoked", revoked));
        });
    }
    @PluginMethod public void models(PluginCall call) {
        perform(call, () -> {
            String id = call.getString("accountId"); JSONObject record = usable(id);
            call.resolve(new JSObject().put("models", fetchModels(id, record.getString("access_token"))));
        });
    }
    private JSArray fetchModels(String id, String token) throws Exception {
        JSONObject response = jsonRequest(API + "/models", "GET", null, token, null, id);
        JSONArray models = response.getJSONArray("models"); if (models.length() > 1000) throw new IOException();
        JSArray output = new JSArray(); Set<String> ids = new HashSet<>();
        for (int i = 0; i < models.length(); i++) {
            JSONObject model = models.getJSONObject(i);
            if (!"list".equals(model.optString("visibility"))) continue;
            String slug = model.getString("slug"), name = model.getString("display_name");
            if (!slug.matches("[A-Za-z0-9._:-]{1,120}") || name.isEmpty() || name.length() > 200) continue;
            output.put(new JSObject().put("id", slug).put("name", name)); ids.add(slug);
        }
        catalog.put(id, ids); return output;
    }
    @PluginMethod public void reconstruct(PluginCall call) {
        perform(call, () -> {
            String id = call.getString("accountId"), model = call.getString("model"), image = call.getString("image"), instructions = call.getString("instructions");
            if (model == null || !model.matches("[A-Za-z0-9._:-]{1,120}") || image == null || image.length() > 12_000_000 ||
                !image.startsWith("data:image/jpeg;base64,") || instructions == null || instructions.isEmpty() || instructions.length() > 12000) throw new SafeFailure("Invalid page or model for ChatGPT reconstruction.");
            byte[] jpeg = android.util.Base64.decode(image.substring(23), android.util.Base64.NO_WRAP);
            if (jpeg.length < 4 || jpeg[0] != (byte) 0xff || jpeg[1] != (byte) 0xd8) throw new SafeFailure("Choose a JPEG scan for ChatGPT reconstruction.");
            JSONObject record = usable(id); String token = record.getString("access_token");
            if (!catalog.containsKey(id)) fetchModels(id, token);
            if (!catalog.get(id).contains(model)) throw new SafeFailure("Refresh this account's model list and choose an available model.");
            JSONArray content = new JSONArray().put(new JSONObject().put("type", "input_image").put("image_url", image).put("detail", "high"))
                .put(new JSONObject().put("type", "input_text").put("text", "Transcribe this page."));
            JSONObject body = new JSONObject().put("model", model).put("store", false).put("stream", true).put("instructions", instructions)
                .put("input", new JSONArray().put(new JSONObject().put("role", "user").put("content", content)));
            // This method issues exactly one paid request. There is no inference retry or provider fallback.
            JSONObject response = inference(id, token, body.toString());
            call.resolve(new JSObject().put("response", response));
        });
    }
    @PluginMethod public void manageUsage(PluginCall call) {
        getActivity().runOnUiThread(() -> {
            try { getActivity().startActivity(new Intent(Intent.ACTION_VIEW, Uri.parse("https://chatgpt.com/settings/usage"))); call.resolve(); }
            catch (Exception error) { call.reject("Open ChatGPT Settings, then Usage, to manage this connection."); }
        });
    }
    private JSONObject usable(String id) throws Exception {
        JSONObject store = loadStore(), record = account(store, id);
        if (stopped.contains(id) || !id.equals(store.optString("activeAccountId"))) throw new SafeFailure("Select and connect this ChatGPT account first.");
        if (!connected(record)) throw new SafeFailure("Sign in to this ChatGPT account again.");
        permitted(record);
        if (record.optLong("expires_at", 0) <= System.currentTimeMillis() + 60_000) {
            if (record.optString("refresh_token").isEmpty()) throw new SafeFailure("Sign in to this ChatGPT account again.");
            if (record.optLong("earliest_refresh_at", 0) > System.currentTimeMillis()) {
                if (record.optLong("expires_at", 0) > System.currentTimeMillis() + 5000) return permitted(record);
                throw new SafeFailure("ChatGPT has not allowed this session to renew yet. Try again shortly.");
            }
            JSONObject tokens;
            try { tokens = jsonRequest(TOKEN, "POST", "application/x-www-form-urlencoded", null,
                ChatGPTProtocol.form(fields("grant_type", "refresh_token", "client_id", id, "refresh_token", record.getString("refresh_token"), "resource", API)), id); }
            catch (HttpFailure error) {
                if (Arrays.asList("invalid_grant", "invalid_refresh_token", "token_expired", "refresh_token_expired", "refresh_token_invalidated", "refresh_token_reused").contains(error.code)) {
                    clearTokens(record); saveStore(store); catalog.remove(id);
                    throw new SafeFailure("This ChatGPT session has ended. Sign in again to reconnect.");
                }
                throw new SafeFailure("The ChatGPT session could not be renewed. Try again later or reconnect the account.");
            }
            catch (Exception error) { throw new SafeFailure("The ChatGPT session could not be renewed. Try again later or reconnect the account."); }
            JSONObject identity = tokens.has("id_token") ? identity(tokens.getString("id_token"), id, null) : null;
            if (identity != null && !record.getString("subject").equals(identity.getString("sub"))) throw new SafeFailure("ChatGPT returned a different account. Sign in again.");
            if (stopped.contains(id)) throw new SafeFailure("This ChatGPT account was signed out.");
            applyTokens(record, tokens, identity, true); saveStore(store);
        }
        return permitted(record);
    }
    private JSONObject permitted(JSONObject record) throws SafeFailure {
        if (!hasScope(record, DIRECT) || !hasScope(record, "resource.invoke")) throw new SafeFailure("ChatGPT plan use is not enabled. Reconnect this account and allow plan usage.");
        return record;
    }
    private static void clearTokens(JSONObject record) {
        for (String key : new String[]{"access_token", "refresh_token", "id_token", "scope", "expires_at", "identity_expires_at", "earliest_refresh_at"}) record.remove(key);
    }
    private void applyTokens(JSONObject record, JSONObject tokens, JSONObject identity, boolean refresh) throws Exception {
        String access = tokens.optString("access_token"), type = tokens.optString("token_type"); long expires = tokens.optLong("expires_in", 0);
        if (!access.isEmpty() && (!"bearer".equalsIgnoreCase(type) || !opaque(access) || expires < 1 || expires > 30L * 24 * 3600)) throw new IOException();
        if (access.isEmpty() && (refresh || identity == null)) throw new IOException();
        String renewable = tokens.optString("refresh_token", refresh ? record.optString("refresh_token") : "");
        if (!renewable.isEmpty() && !opaque(renewable)) throw new IOException();
        String scopes = tokens.optString("scope", refresh ? record.optString("scope") : ""); if (scopes.length() > 4096) throw new IOException();
        record.put("access_token", access).put("refresh_token", renewable).put("scope", scopes).put("expires_at", System.currentTimeMillis() + expires * 1000);
        record.put("earliest_refresh_at", refreshTime(tokens.opt("earliest_refresh_at")));
        if (identity != null) {
            record.put("subject", identity.getString("sub")).put("issuer", ISSUER).put("id_token", tokens.getString("id_token")).put("identity_expires_at", identity.getLong("exp") * 1000);
            String email = identity.optString("email", ""); record.put("email", email.length() <= 320 ? email : "");
        }
    }
    private static long refreshTime(Object value) throws Exception {
        if (value == null || value == JSONObject.NULL) return 0;
        if (value instanceof Number) {
            long seconds = ((Number) value).longValue(); if (seconds < 0 || seconds > Long.MAX_VALUE / 1000) throw new IOException(); return seconds * 1000;
        }
        if (value instanceof String) {
            String text = (String) value;
            for (String format : new String[]{"yyyy-MM-dd'T'HH:mm:ss.SSSX", "yyyy-MM-dd'T'HH:mm:ssX"}) {
                java.text.SimpleDateFormat parser = new java.text.SimpleDateFormat(format, java.util.Locale.US);
                parser.setLenient(false); parser.setTimeZone(java.util.TimeZone.getTimeZone("UTC")); java.text.ParsePosition position = new java.text.ParsePosition(0);
                java.util.Date date = parser.parse(text, position); if (date != null && position.getIndex() == text.length()) return date.getTime();
            }
        }
        throw new IOException();
    }
    private static boolean opaque(String value) { return value != null && !value.isEmpty() && value.length() <= 32768 && value.matches("[\\x21-\\x7E]+"); }
    private static boolean hasScope(JSONObject record, String scope) { return Arrays.asList(record.optString("scope").split("\\s+")).contains(scope); }
    private static boolean connected(JSONObject record) {
        return !record.optString("subject").isEmpty() && ((!record.optString("access_token").isEmpty() &&
            (record.optLong("expires_at") > System.currentTimeMillis() || !record.optString("refresh_token").isEmpty())) ||
            (!record.optString("id_token").isEmpty() && record.optLong("identity_expires_at") > System.currentTimeMillis()));
    }
    private JSONObject identity(String token, String clientId, String nonce) throws Exception {
        if (token.length() > 32768) throw new IOException();
        String[] parts = token.split("\\.", -1); if (parts.length != 3) throw new IOException();
        JSONObject header = new JSONObject(new String(ChatGPTProtocol.base64url(parts[0]), StandardCharsets.UTF_8));
        if (!"RS256".equals(header.optString("alg")) || header.has("crit") || header.optString("kid").isEmpty()) throw new IOException();
        JSONObject key = signingKey(header.getString("kid"));
        BigInteger modulus = new BigInteger(1, ChatGPTProtocol.base64url(key.getString("n")));
        if (modulus.bitLength() < 2048 || modulus.bitLength() > 8192) throw new IOException();
        Signature verifier = Signature.getInstance("SHA256withRSA");
        verifier.initVerify(KeyFactory.getInstance("RSA").generatePublic(new RSAPublicKeySpec(modulus, new BigInteger(1, ChatGPTProtocol.base64url(key.getString("e"))))));
        verifier.update((parts[0] + "." + parts[1]).getBytes(StandardCharsets.US_ASCII));
        if (!verifier.verify(ChatGPTProtocol.base64url(parts[2]))) throw new IOException();
        JSONObject claims = new JSONObject(new String(ChatGPTProtocol.base64url(parts[1]), StandardCharsets.UTF_8));
        long now = System.currentTimeMillis() / 1000;
        if (!ISSUER.equals(claims.optString("iss")) || claims.getLong("exp") <= now - 5 || claims.getLong("iat") > now + 5 || claims.getLong("iat") < 1 ||
            (claims.has("nbf") && claims.getLong("nbf") > now + 5) || claims.optString("sub").isEmpty() || claims.getString("sub").length() > 512) throw new IOException();
        Object audience = claims.get("aud"); boolean matches = clientId.equals(audience);
        if (audience instanceof JSONArray) {
            JSONArray audiences = (JSONArray) audience;
            for (int i = 0; i < audiences.length(); i++) if (clientId.equals(audiences.get(i))) matches = true;
            if (audiences.length() > 1 && !clientId.equals(claims.optString("azp"))) throw new IOException();
        }
        if (!matches || (claims.has("azp") && !clientId.equals(claims.optString("azp"))) || (nonce != null && !ChatGPTProtocol.equal(nonce, claims.optString("nonce", null)))) throw new IOException();
        return claims;
    }
    private JSONObject signingKey(String kid) throws Exception {
        for (int attempt = 0; attempt < 2; attempt++) {
            if (cachedKeys == null || System.currentTimeMillis() - keysAt > 300_000 || attempt == 1) {
                cachedKeys = jsonRequest(JWKS, "GET", null, null, null, null); keysAt = System.currentTimeMillis();
            }
            JSONArray keys = cachedKeys.getJSONArray("keys"); if (keys.length() > 100) throw new IOException();
            JSONObject found = null;
            for (int i = 0; i < keys.length(); i++) {
                JSONObject key = keys.getJSONObject(i);
                if (kid.equals(key.optString("kid")) && "RSA".equals(key.optString("kty")) && "RS256".equals(key.optString("alg")) && "sig".equals(key.optString("use", "sig"))) {
                    if (found != null) throw new IOException(); found = key;
                }
            }
            if (found != null) return found;
        }
        throw new IOException();
    }
    private JSONObject inference(String id, String token, String body) throws Exception {
        HttpURLConnection connection = connection(API + "/responses", "POST", "application/json", token, body, id);
        try {
            int status = connection.getResponseCode();
            if (status < 200 || status >= 300) throw apiFailure(status, errorCode(connection));
            String type = connection.getContentType(); if (type == null || !type.toLowerCase(java.util.Locale.ROOT).startsWith("text/event-stream")) throw new IOException();
            try (InputStream input = new BufferedInputStream(connection.getInputStream())) {
                StringBuilder event = new StringBuilder(); int total = 0; String line;
                while ((line = ChatGPTProtocol.line(input, 1_048_576)) != null) {
                    total += line.length() + 1; if (total > 12_000_000 || stopped.contains(id)) throw new IOException();
                    if (line.isEmpty()) {
                        if (event.length() > 0) {
                            JSONObject complete = eventResult(event.toString()); if (complete != null) return complete;
                            event.setLength(0);
                        }
                    } else if (line.startsWith("data:")) {
                        String value = line.substring(5); if (value.startsWith(" ")) value = value.substring(1);
                        if (event.length() > 0) event.append('\n'); event.append(value);
                        if (event.length() > 4_000_000) throw new IOException();
                    }
                }
                // An unterminated event or [DONE] is not proof of a completed response.
                throw new SafeFailure("ChatGPT stopped before completing this page. Your scan is preserved; retrying may use more of your plan.");
            }
        } finally { closeConnection(id, connection); }
    }
    private JSONObject eventResult(String data) throws Exception {
        if ("[DONE]".equals(data)) return null;
        JSONObject event = new JSONObject(data); String type = event.optString("type");
        if ("error".equals(type) || "response.failed".equals(type)) {
            JSONObject response = event.optJSONObject("response"), error = response == null ? event.optJSONObject("error") : response.optJSONObject("error");
            throw apiFailure(200, error == null ? event.optString("code") : error.optString("code"));
        }
        if ("response.incomplete".equals(type)) throw new SafeFailure("ChatGPT could not finish this page. Your scan is preserved; try another model.");
        if (!"response.completed".equals(type)) return null;
        JSONObject response = event.getJSONObject("response");
        if (!"completed".equals(response.optString("status")) || (!response.isNull("error") && response.has("error")) ||
            (!response.isNull("incomplete_details") && response.has("incomplete_details"))) throw new IOException();
        JSONArray output = response.getJSONArray("output"), safeOutput = new JSONArray(); int textLength = 0;
        for (int i = 0; i < output.length(); i++) {
            JSONObject item = output.getJSONObject(i);
            if ("reasoning".equals(item.optString("type"))) continue;
            if (!"message".equals(item.optString("type")) || !"completed".equals(item.optString("status", "completed"))) throw new IOException();
            JSONArray content = item.getJSONArray("content"), safeContent = new JSONArray();
            for (int j = 0; j < content.length(); j++) {
                JSONObject part = content.getJSONObject(j);
                if ("refusal".equals(part.optString("type"))) throw new SafeFailure("ChatGPT declined this page. Try on-device OCR.");
                if (!"output_text".equals(part.optString("type"))) throw new IOException();
                textLength += part.getString("text").length();
                safeContent.put(new JSONObject().put("type", "output_text").put("text", part.getString("text")));
            }
            safeOutput.put(new JSONObject().put("type", "message").put("status", "completed").put("role", "assistant").put("content", safeContent));
        }
        if (textLength < 1 || textLength > 24000) throw new SafeFailure("The ChatGPT result is empty or too long. Your scan is preserved.");
        return new JSONObject().put("status", "completed").put("output", safeOutput);
    }
    private SafeFailure apiFailure(int status, String code) {
        if ("subscription_sharing_user_not_eligible".equals(code)) return new SafeFailure("ChatGPT plan use is unavailable for this account, workspace or policy. Choose another supported connection.");
        if ("subscription_sharing_unsupported_capability".equals(code)) return new SafeFailure("This ChatGPT model does not support the requested page input. Choose another model.");
        if (status == 429 || "subscription_sharing_usage_limit_exceeded".equals(code) || "subscription_sharing_usage_unavailable".equals(code))
            return new SafeFailure("ChatGPT plan usage is unavailable or has reached its limit. Open Manage Usage in ChatGPT or try again later.");
        if (status == 401 || status == 403) return new SafeFailure("ChatGPT access is unavailable. Reconnect your account and check its plan permissions.");
        return new SafeFailure("ChatGPT could not finish this page. Your scan is preserved; retrying may use more of your plan.");
    }
    private HttpURLConnection connection(String target, String method, String type, String token, String body, String id) throws Exception {
        if (!target.equals(TOKEN) && !target.equals(REVOKE) && !target.equals(DISCOVERY) && !target.equals(JWKS) && !target.equals(API + "/models") && !target.equals(API + "/responses")) throw new IOException();
        HttpURLConnection result = (HttpURLConnection) new URL(target).openConnection();
        result.setInstanceFollowRedirects(false); result.setConnectTimeout(20000); result.setReadTimeout(30000); result.setRequestMethod(method);
        result.setUseCaches(false); result.setRequestProperty("Accept", target.endsWith("/responses") ? "text/event-stream" : "application/json");
        if (token != null) result.setRequestProperty("Authorization", "Bearer " + token);
        if (type != null) result.setRequestProperty("Content-Type", type);
        synchronized (requestGate) {
            if (destroyed || (id != null && stopped.contains(id))) { result.disconnect(); throw new SafeFailure("This ChatGPT connection was closed."); }
            allRequests.add(result); if (id != null) requests.put(id, result);
            requestDeadlines.put(result, deadlines.schedule(result::disconnect, target.endsWith("/responses") ? 180 : 45, TimeUnit.SECONDS));
        }
        try {
            if (body != null) {
                byte[] bytes = body.getBytes(StandardCharsets.UTF_8); result.setDoOutput(true); result.setFixedLengthStreamingMode(bytes.length);
                if (destroyed || (id != null && stopped.contains(id))) throw new IOException();
                try (java.io.OutputStream output = result.getOutputStream()) {
                    for (int offset = 0; offset < bytes.length; offset += 8192) {
                        if (destroyed || (id != null && stopped.contains(id))) throw new IOException();
                        output.write(bytes, offset, Math.min(8192, bytes.length - offset));
                    }
                }
            }
            return result;
        } catch (Exception error) { closeConnection(id, result); throw error; }
    }
    private void closeConnection(String id, HttpURLConnection connection) {
        ScheduledFuture<?> deadline = requestDeadlines.remove(connection); if (deadline != null) deadline.cancel(false);
        connection.disconnect(); allRequests.remove(connection); if (id != null) requests.remove(id, connection);
    }
    private String errorCode(HttpURLConnection connection) {
        try (InputStream input = connection.getErrorStream()) {
            if (input == null) return null;
            JSONObject body = new JSONObject(new String(bounded(input, 65536), StandardCharsets.UTF_8));
            JSONObject error = body.optJSONObject("error"); String code = error == null ? body.optString("error") : error.optString("code");
            return code.matches("[a-z_]{1,100}") ? code : null;
        } catch (Exception ignored) { return null; }
    }
    private byte[] requestBytes(String target, String method, String type, String token, String body, String id) throws Exception {
        HttpURLConnection connection = connection(target, method, type, token, body, id);
        try {
            int status = connection.getResponseCode(); if (status < 200 || status >= 300) throw new HttpFailure(status, errorCode(connection));
            try (InputStream input = connection.getInputStream()) { return bounded(input, 1_000_000); }
        } finally { closeConnection(id, connection); }
    }
    private JSONObject jsonRequest(String target, String method, String type, String token, String body, String id) throws Exception {
        return new JSONObject(new String(requestBytes(target, method, type, token, body, id), StandardCharsets.UTF_8));
    }
    private static byte[] bounded(InputStream input, int limit) throws Exception {
        ByteArrayOutputStream out = new ByteArrayOutputStream(); byte[] buffer = new byte[8192]; int count;
        while ((count = input.read(buffer)) != -1) { if (out.size() + count > limit) throw new IOException(); out.write(buffer, 0, count); }
        return out.toByteArray();
    }
    private static Map<String, String> fields(String... fields) {
        Map<String, String> result = new LinkedHashMap<>(); for (int i = 0; i < fields.length; i += 2) result.put(fields[i], fields[i + 1]); return result;
    }
    private JSONObject account(JSONObject store, String id) throws Exception {
        JSONObject account = findAccount(store, id); if (account == null) throw new SafeFailure("Choose a saved ChatGPT account."); return account;
    }
    private JSONObject findAccount(JSONObject store, String id) throws Exception {
        if (id == null) return null;
        JSONArray accounts = store.getJSONArray("accounts");
        for (int i = 0; i < accounts.length(); i++) if (id.equals(accounts.getJSONObject(i).getString("id"))) return accounts.getJSONObject(i);
        return null;
    }
    private JSObject snapshot(JSONObject store) throws Exception {
        JSArray accounts = new JSArray(); JSONArray records = store.getJSONArray("accounts");
        for (int i = 0; i < records.length(); i++) {
            JSONObject record = records.getJSONObject(i); boolean connected = connected(record) && !stopped.contains(record.getString("id"));
            accounts.put(new JSObject().put("id", record.getString("id")).put("email", record.optString("email"))
                .put("label", record.getString("label")).put("connected", connected)
                .put("planEnabled", connected && hasScope(record, DIRECT) && hasScope(record, "resource.invoke")));
        }
        JSObject result = new JSObject().put("accounts", accounts);
        if (!store.optString("activeAccountId").isEmpty()) result.put("activeAccountId", store.getString("activeAccountId"));
        return result;
    }
    private SecretKey storageKey() throws Exception {
        KeyStore keys = KeyStore.getInstance("AndroidKeyStore"); keys.load(null);
        if (!keys.containsAlias(KEY_ALIAS)) {
            KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
            generator.init(new KeyGenParameterSpec.Builder(KEY_ALIAS, KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).setKeySize(256).build());
            generator.generateKey();
        }
        return (SecretKey) keys.getKey(KEY_ALIAS, null);
    }
    private AtomicFile storage() { return new AtomicFile(new File(getContext().getNoBackupFilesDir(), "chatgpt-accounts-v1.enc")); }
    private JSONObject loadStore() throws Exception {
        AtomicFile file = storage();
        if (!file.getBaseFile().exists() && !new File(file.getBaseFile().getPath() + ".bak").exists()) {
            JSONObject fresh = new JSONObject().put("host", "urn:uuid:" + UUID.randomUUID()).put("accounts", new JSONArray()); saveStore(fresh); return fresh;
        }
        byte[] encrypted;
        try (InputStream input = file.openRead()) { encrypted = bounded(input, 2_000_000); }
        if (encrypted.length < 29 || encrypted[0] != 1) throw new IOException();
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
        cipher.init(Cipher.DECRYPT_MODE, storageKey(), new GCMParameterSpec(128, Arrays.copyOfRange(encrypted, 1, 13))); cipher.updateAAD(AAD);
        JSONObject store = new JSONObject(new String(cipher.doFinal(Arrays.copyOfRange(encrypted, 13, encrypted.length)), StandardCharsets.UTF_8));
        if (!store.getString("host").matches("urn:uuid:[0-9a-f-]{36}") || store.getJSONArray("accounts").length() > 50) throw new IOException();
        return store;
    }
    private void saveStore(JSONObject store) throws Exception {
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding"); cipher.init(Cipher.ENCRYPT_MODE, storageKey()); cipher.updateAAD(AAD);
        byte[] content = store.toString().getBytes(StandardCharsets.UTF_8); if (content.length > 1_900_000) throw new IOException();
        byte[] encrypted = cipher.doFinal(content), iv = cipher.getIV(); if (iv.length != 12) throw new IOException();
        AtomicFile file = storage(); FileOutputStream out = null;
        try {
            out = file.startWrite(); out.write(1); out.write(iv); out.write(encrypted); file.finishWrite(out); out = null;
            file.getBaseFile().setReadable(false, false); file.getBaseFile().setWritable(false, false);
            file.getBaseFile().setReadable(true, true); file.getBaseFile().setWritable(true, true);
        } catch (Exception error) { if (out != null) file.failWrite(out); throw error; }
    }
    @Override protected void handleOnDestroy() {
        destroyed = true; Attempt attempt = pending; if (attempt != null) finish(attempt, null, "ChatGPT sign-in was closed. Start again.");
        for (HttpURLConnection request : allRequests) request.disconnect();
        listener.shutdownNow(); serial.shutdown(); deadlines.shutdownNow();
    }
}
