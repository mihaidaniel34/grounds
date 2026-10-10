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
