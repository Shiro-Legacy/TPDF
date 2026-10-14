package com.tandempdf.mobile;
import java.io.ByteArrayInputStream;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.Map;

public final class ChatGPTProtocolTest {
    private static int checks;
    private interface Operation { void run() throws Exception; }
    private static void check(boolean value, String name) { if (!value) throw new AssertionError(name); checks++; }
    private static void rejects(String name, Operation action) throws Exception {
        try { action.run(); } catch (Exception expected) { checks++; return; }
        throw new AssertionError("Accepted " + name);
    }
    public static void main(String[] args) throws Exception {
        check(ChatGPTProtocol.challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk").equals("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"),"RFC7636 PKCE fixture");
        String state = ChatGPTProtocol.random(32);
        check(state.length()==43 && state.matches("[A-Za-z0-9_-]+"),"state entropy encoding");
        check(!state.equals(ChatGPTProtocol.random(32)),"fresh authorization state");
        check(ChatGPTProtocol.equal(state,state),"same state accepted");
        check(!ChatGPTProtocol.equal(state,state+"x") && !ChatGPTProtocol.equal(state,null) && !ChatGPTProtocol.equal(null,null),"wrong or absent state rejected");
        Map<String,String> fields=ChatGPTProtocol.query("code=one%2Buse%2Fcode&state="+state+"&client_id=oaiapp_test");
        check(fields.get("code").equals("one+use/code") && ChatGPTProtocol.equal(state,fields.get("state")),"callback encoded values");
        for(String bad:new String[]{"state=a&state=b","state=a&%73tate=b","code=x&code=x","=value","state","state=%GG"})rejects("ambiguous or malformed callback",()->ChatGPTProtocol.query(bad));
        rejects("oversize callback",()->ChatGPTProtocol.query("x="+"a".repeat(16384)));
        Map<String,String> form=new LinkedHashMap<>();form.put("redirect_uri","http://127.0.0.1:12345/auth/callback");form.put("code","value+&= 中文");
        check(ChatGPTProtocol.query(ChatGPTProtocol.form(form)).equals(form),"form encoding round trip");
        check(Arrays.equals(ChatGPTProtocol.base64url("APv-_w"),new byte[]{0,(byte)251,(byte)254,(byte)255}),"URL-safe token decode");
        for(String bad:new String[]{"","a","YQ=","YQ==","Y+","Y/","YR","YWJ","YQ\n"})rejects("noncanonical token segment",()->ChatGPTProtocol.base64url(bad));
        rejects("oversize token segment",()->ChatGPTProtocol.base64url("a".repeat(32769)));
        ByteArrayInputStream stream=new ByteArrayInputStream("data: first\r\n\ndata: second\n".getBytes(StandardCharsets.UTF_8));
        check(ChatGPTProtocol.line(stream,100).equals("data: first"),"CRLF SSE line");
        check(ChatGPTProtocol.line(stream,100).equals(""),"SSE boundary retained");
        check(ChatGPTProtocol.line(stream,100).equals("data: second"),"LF SSE line");
        check(ChatGPTProtocol.line(stream,100)==null,"stream EOF distinct from empty boundary");
        rejects("oversized SSE line",()->ChatGPTProtocol.line(new ByteArrayInputStream("abcd\n".getBytes(StandardCharsets.UTF_8)),3));
        check(ChatGPTProtocol.line(new ByteArrayInputStream("abc\n".getBytes(StandardCharsets.UTF_8)),3).equals("abc"),"exact line limit accepted");
        System.out.println("PASS: "+checks+" Android PKCE, state, callback encoding and bounded stream helper checks (production class; no network).");
    }
}
