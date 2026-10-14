package com.tandempdf.mobile;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.net.URLDecoder;
import java.net.URLEncoder;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.SecureRandom;
import java.util.LinkedHashMap;
import java.util.Map;

/** Pure protocol helpers, kept separate so callback and stream limits can be tested on the JVM. */
final class ChatGPTProtocol {
    private ChatGPTProtocol() {}
    static String random(int bytes) {
        byte[] value = new byte[bytes]; new SecureRandom().nextBytes(value);
        return encode(value);
    }
    static String challenge(String verifier) throws Exception {
        return encode(MessageDigest.getInstance("SHA-256").digest(verifier.getBytes(StandardCharsets.US_ASCII)));
    }
    static boolean equal(String expected, String actual) {
        return expected != null && actual != null && MessageDigest.isEqual(
            expected.getBytes(StandardCharsets.UTF_8), actual.getBytes(StandardCharsets.UTF_8));
    }
    static String form(Map<String, String> values) throws Exception {
        StringBuilder result = new StringBuilder();
        for (Map.Entry<String, String> item : values.entrySet()) {
            if (result.length() > 0) result.append('&');
            result.append(URLEncoder.encode(item.getKey(), "UTF-8")).append('=')
                .append(URLEncoder.encode(item.getValue(), "UTF-8"));
        }
        return result.toString();
    }
    static Map<String, String> query(String query) throws Exception {
        if (query == null || query.length() > 16384) throw new IOException();
        Map<String, String> fields = new LinkedHashMap<>();
        for (String part : query.split("&")) {
            int equals = part.indexOf('=');
            if (equals < 1) throw new IOException();
            String name = URLDecoder.decode(part.substring(0, equals), "UTF-8");
            String value = URLDecoder.decode(part.substring(equals + 1), "UTF-8");
            if (fields.put(name, value) != null) throw new IOException();
        }
        return fields;
    }
    static byte[] base64url(String value) throws Exception {
        if (value == null || value.length() > 32768 || value.length() % 4 == 1 || !value.matches("[A-Za-z0-9_-]+")) throw new IOException();
        ByteArrayOutputStream result = new ByteArrayOutputStream(); int accumulator = 0, bits = 0;
        for (int i = 0; i < value.length(); i++) {
            accumulator = (accumulator << 6) | ALPHABET.indexOf(value.charAt(i)); bits += 6;
            if (bits >= 8) { bits -= 8; result.write((accumulator >>> bits) & 255); }
        }
        if (bits != 0 && (accumulator & ((1 << bits) - 1)) != 0) throw new IOException();
        return result.toByteArray();
    }
    private static final String ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    private static String encode(byte[] input) {
        StringBuilder output = new StringBuilder(); int accumulator = 0, bits = 0;
        for (byte value : input) {
            accumulator = (accumulator << 8) | (value & 255); bits += 8;
            while (bits >= 6) { bits -= 6; output.append(ALPHABET.charAt((accumulator >>> bits) & 63)); }
        }
        if (bits > 0) output.append(ALPHABET.charAt((accumulator << (6 - bits)) & 63));
        return output.toString();
    }
    static String line(InputStream input, int limit) throws IOException {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        int next;
        while ((next = input.read()) != -1) {
            if (next == '\n') break;
            if (out.size() >= limit) throw new IOException();
            out.write(next);
        }
        if (next == -1 && out.size() == 0) return null;
        byte[] bytes = out.toByteArray();
        int length = bytes.length;
        if (length > 0 && bytes[length - 1] == '\r') length--;
        return new String(bytes, 0, length, StandardCharsets.UTF_8);
    }
}
