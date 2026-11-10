// `cmd = cmd;` reads the value from before it; this used to overflow the stack
class SelfAssignment {
    void built(String file) throws Exception {
        String cmd = "rm -rf " + file;
        cmd = cmd;
        Runtime.getRuntime().exec(cmd); // expect
    }

    void constant() throws Exception {
        String cmd = "date";
        cmd = cmd;
        Runtime.getRuntime().exec(cmd);
    }
}
