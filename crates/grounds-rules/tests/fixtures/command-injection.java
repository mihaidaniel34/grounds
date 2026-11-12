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
        new ProcessBuilder("bash", "-e", "-c", "cat " + file); // expect
        new ProcessBuilder("cmd", "/c", "type", "x " + file); // expect
        // an option's value is no command
        new ProcessBuilder("bash", "-o", "pipefail", "-c", "cat " + file); // expect
        new ProcessBuilder("bash", "-c", "-o", "pipefail", "cat " + file); // expect
        // (a -O/+O shopt name is not read: missed, as for one bash lacks)
        new ProcessBuilder("bash", "+O", "extglob", "-c", "cat " + file);
    }
    // through env: the shell still gets the command string
    void env(String file) throws Exception {
        new ProcessBuilder("env", "sh", "-c", "cat " + file); // expect
        new ProcessBuilder("/usr/bin/env", "bash", "-c", "cat " + file); // expect
        new ProcessBuilder("env", "-i", "--", "LANG=C", "sh", "-c", "cat " + file); // expect
        new ProcessBuilder("env", "--", "LANG=C", "sh", "-c", "cat " + file); // expect
        // the options end at an assignment: the program is "--"
        new ProcessBuilder("env", "-i", "LANG=C", "--", "sh", "-c", "cat " + file);
        new ProcessBuilder("env", "-S", "sh -c", "cat " + file);
    }
    // options as the shell reads them: -C is noclobber, -- ends the options,
    // --rcfile takes "-c" as its file, an option not known says nothing
    void options(String x) throws Exception {
        new ProcessBuilder("bash", "-C", "script-" + x);
        new ProcessBuilder("bash", "--", "-c", "arg-" + x);
        new ProcessBuilder("bash", "--rcfile", "-c", "script-" + x);
        new ProcessBuilder("bash", "--unknown-option", "-c", "cat " + x);
        new ProcessBuilder("bash", "-c", "--", "cat " + x); // expect
        new ProcessBuilder("bash", "-xc", "cat " + x); // expect
        // -n reads the commands without running them
        new ProcessBuilder("bash", "-n", "-c", "echo " + x);
        new ProcessBuilder("sh", "-n", "-c", "echo " + x);
        new ProcessBuilder("bash", "-nc", "echo " + x);
        new ProcessBuilder("bash", "-c", "-n", "echo " + x);
        new ProcessBuilder("bash", "-o", "noexec", "-c", "echo " + x);
        new ProcessBuilder("zsh", "-onoexec", "-c", "echo " + x);
        // zsh takes an option's name attached
        new ProcessBuilder("zsh", "-onounset", "-c", "echo " + x); // expect
        new ProcessBuilder("zsh", "-oerrexit", "-c", "echo " + x); // expect
        new ProcessBuilder("bash", "-onounset", "-c", "echo " + x);
        new ProcessBuilder("zsh", "-o", "errexit", "-c", "echo " + x); // expect
        new ProcessBuilder("bash", "-o", "pipefail", "-c", "echo " + x); // expect
        // zsh: unsetting exec runs nothing, in every spelling
        new ProcessBuilder("zsh", "+oexec", "-c", "echo " + x);
        new ProcessBuilder("zsh", "+o", "exec", "-c", "echo " + x);
        new ProcessBuilder("zsh", "-oNO_EXEC", "-c", "echo " + x);
        new ProcessBuilder("zsh", "+oNO_EXEC", "-c", "echo " + x); // expect
        // an option name the shell lacks: it refuses before running anything
        new ProcessBuilder("zsh", "-oTHIS_OPTION_DOES_NOT_EXIST", "-c", "echo " + x);
        new ProcessBuilder("bash", "-o", "nosuchoption", "-c", "echo " + x);
        // zsh: -b ends the options, a later -c is a script's name
        new ProcessBuilder("zsh", "-b", "-c", "echo " + x);
        new ProcessBuilder("zsh", "-eb", "-c", "echo " + x);
        new ProcessBuilder("zsh", "-bc", "echo " + x); // expect
        new ProcessBuilder("zsh", "+b", "-c", "echo " + x);
        new ProcessBuilder("zsh", "+eb", "-c", "echo " + x);
        // bash's -O names (and zsh's letter -O) are not read
        new ProcessBuilder("bash", "-O", "nosuch", "-c", "echo " + x);
        new ProcessBuilder("bash", "+O", "nosuch", "-c", "echo " + x);
        // sh may be dash: only what dash and bash both take
        new ProcessBuilder("sh", "-o", "pipefail", "-c", "echo " + x);
        new ProcessBuilder("sh", "-O", "extglob", "-c", "echo " + x);
        new ProcessBuilder("sh", "--norc", "-c", "echo " + x);
        new ProcessBuilder("sh", "-k", "-c", "echo " + x);
        new ProcessBuilder("sh", "-o", "interactive-comments", "-c", "echo " + x);
        new ProcessBuilder("sh", "-e", "-o", "nounset", "-c", "echo " + x); // expect
        new ProcessBuilder("dash", "-x", "-c", "echo " + x); // expect
        // not every dash takes privileged
        new ProcessBuilder("dash", "-o", "privileged", "-c", "echo " + x);
        new ProcessBuilder("dash", "+o", "privileged", "-c", "echo " + x);
        new ProcessBuilder("dash", "-c", "-o", "privileged", "echo " + x);
        // ksh's options are not checked
        new ProcessBuilder("ksh", "-c", "echo " + x);
    }
    // the program is the first element: echo prints "sh -c cat <file>"
    void notAShell(String file) throws Exception {
        new ProcessBuilder("/bin/echo", "sh", "-c", "cat " + file);
        Runtime.getRuntime().exec(new String[] {"ssh-keygen", "-C", "sh -c " + file, "-f", "/tmp/" + file});
    }
    // after the command string come the script's own arguments
    void positional(String file) throws Exception {
        new ProcessBuilder("sh", "-c", "cat \"$1\"", "sh", "./" + file);
    }
    // one string: Runtime.exec splits it on spaces
    void single(String file) throws Exception {
        Runtime.getRuntime().exec("tar -xf " + file, null, null); // expect
    }
}
