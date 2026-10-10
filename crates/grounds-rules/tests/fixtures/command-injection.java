class A {
    void m(String file) throws Exception {
        Runtime.getRuntime().exec("rm -rf " + file); // expect
        new ProcessBuilder("sh", "-c", "cat " + file); // expect
        Runtime.getRuntime().exec(new String[] {"rm", file});
    }
}
