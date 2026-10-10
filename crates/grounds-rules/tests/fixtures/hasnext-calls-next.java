import java.util.*;
class Bad implements Iterator<String> {
    String peeked;
    public boolean hasNext() { return next() != null; } // expect
    public String next() { if (peeked == null) throw new NoSuchElementException(); return peeked; }
}
class Good implements Iterator<String> {
    Iterator<String> d;
    public boolean hasNext() { return d.hasNext(); }
    public String next() { return d.next(); }
}
