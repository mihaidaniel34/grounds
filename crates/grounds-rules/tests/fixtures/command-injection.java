class A {
    void m(String file) throws Exception {
        Runtime.getRuntime().exec("rm -rf " + file); // expect
        new ProcessBuilder("sh", "-c", "cat " + file); // expect
        Runtime.getRuntime().exec(new String[] {"rm", file});
    }
}
class Argv {
    static final String SHELL = "/bin/bash";
    // one element of an argument vector is one argument: no shell splits it
    void vector(String python, java.nio.file.Path dir, String version) throws Exception {
        new ProcessBuilder(python, "-m", "pip", "install", "--target=" + dir, "pkg==" + version);
        Runtime.getRuntime().exec(new String[] {"tar", "-C", "/tmp/" + dir, "-xf", "a.tar"});
        new ProcessBuilder(java.util.List.of("git", "clone", "https://host/" + version));
    }
    // a shell parses its command string again
    void shell(String file) throws Exception {
        Runtime.getRuntime().exec(new String[] {"/bin/sh", "-c", "cat " + file}); // expect
        new ProcessBuilder(java.util.Arrays.asList(SHELL, "-c", "ls " + file)); // expect
        new ProcessBuilder("cmd.exe", "/c", "type " + file); // expect
        new ProcessBuilder("bash", "-lc", "cat " + file); // expect
        new ProcessBuilder("sh", "-c", "date");
        String shell = "/bin/sh";
        new ProcessBuilder(shell, "-c", "cat " + file); // expect
    }
    // one string: Runtime.exec splits it on spaces
    void single(String file) throws Exception {
        Runtime.getRuntime().exec("tar -xf " + file, null, null); // expect
    }
}
