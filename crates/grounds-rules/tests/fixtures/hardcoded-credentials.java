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
    // names of settings, query fragments and labels are not secrets
    static final String ENV_ACCESS_KEY = "spas_accessKey";
    static final String LLM_API_KEY = "SKILL_SCANNER_LLM_API_KEY";
    static final String GRANT_TOKEN = "client_credentials";
    static final String SECRET_PARAM_TOKEN = "&client_secret=";
    static final String FORM_TOKEN = "password=";
    static final String QUERY_SECRET = "?client_secret=abc123"; // expect
    static final String DB_PASSWORD = "correct_horse_battery_staple"; // expect
    static final String SIGNING_SECRET = "c2VjcmV0LWtleS0xMjM0NTY3ODk="; // expect
    static final String DING_TALK_PASSWORD = "$t('password')";
}
