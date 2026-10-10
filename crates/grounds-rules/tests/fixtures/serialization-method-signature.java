import java.io.*;
class A implements Serializable {
    public void writeObject(ObjectOutputStream out) throws IOException { } // expect
    private void readObject(ObjectInputStream in) throws IOException { }
    static Object readResolve() { return null; } // expect
}
class B implements Serializable {
    private Object readResolve() { return this; }
}
