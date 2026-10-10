import java.io.*;
class A {
    Object m(InputStream in) throws Exception {
        return new ObjectInputStream(in).readObject(); // expect
    }
}
