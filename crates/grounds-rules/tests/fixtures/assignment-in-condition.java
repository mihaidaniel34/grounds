import java.io.*;
class A {
    void m(boolean done, BufferedReader r) throws IOException {
        if (done = true) { } // expect
        while (done = false) { } // expect
        String line;
        while ((line = r.readLine()) != null) { }
        if (done == true) { }
    }
}
