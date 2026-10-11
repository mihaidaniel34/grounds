class A {
    private String password = "hunter22"; // expect
    private static final String API_KEY = "sk-live-abcdef123456"; // expect
    String dbPassword = "${DB_PASSWORD}";
    String passwordField = "password";
    String token = "";
    void m() {
        String secret = "s3cr3tValue"; // expect
        String passwordLabel = "Enter password";
    }
}
class Keys {
    static final String CREATE_ACCESS_KEY = "AccessKey.Create";
    static final String TLS_TRUST_PWD = ".certPrivateKeyPassword";
    static final String PROVISION_SECRET = "provisionDeviceSecret";
    static final String AUTH_TOKEN = "Authorization";
    static final String CSRF_HEADER_TOKEN = "X-CSRF-Token";
}
