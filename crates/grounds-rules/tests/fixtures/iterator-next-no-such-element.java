import java.util.*;
class Bad implements Iterator<String> {
    int i;
    public boolean hasNext() { return i < 3; }
    public String next() { i++; return "x"; } // expect
}
class Good implements Iterator<String> {
    int i;
    public boolean hasNext() { return i < 3; }
    public String next() { if (!hasNext()) throw new NoSuchElementException(); i++; return "x"; }
}
class Delegate implements Iterator<String> {
    Iterator<String> d;
    public boolean hasNext() { return d.hasNext(); }
    public String next() { return d.next(); }
}
