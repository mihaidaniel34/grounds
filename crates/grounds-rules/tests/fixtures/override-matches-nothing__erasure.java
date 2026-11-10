import java.util.Date;
import java.util.List;

// A method overrides one whose signature's erasure is its own (JLS 8.4.2,
// 8.4.8.1): type variables erase to their first bound, also through the
// supertype's type arguments and a generic method's bounds.
interface Ops<A> { Object get(A array, int i); }

class ObjOps<T> implements Ops<T[]> {
    @Override public Object get(Object[] array, int i) { return array[i]; }
}

abstract class Adapter<T> { abstract void write(StringBuilder out, T value); }

class DateAdapter<T extends Date> extends Adapter<T> {
    @Override void write(StringBuilder out, Date value) { }
}

interface Repo<E> { <S extends E> S save(S entity); }

class Book { }

abstract class BookRepo implements Repo<Book> {
    @Override public Book save(Book entity) { return entity; }
}

interface Numbers<T extends Number & Comparable<T>> { void in(T... values); }

abstract class Ints implements Numbers<Integer> {
    @Override public void in(Integer... values) { }
}

abstract class Bounded<N extends Number & Comparable<N>> implements Numbers<N> {
    @Override public void in(Number... values) { }
}

class RawList<T> {
    void take(List<T> l) { }
}

class RawTaker extends RawList<String> {
    @Override void take(List l) { }
}

class NotOverrides<T extends Date> extends Adapter<T> {
    @Override void write(StringBuilder out, T value) { }
    @Override void write(StringBuilder out, Number value) { } // expect
    @Override void write(String out, Date value) { } // expect
}

class Clash<T extends Date> extends Adapter<Date> {
    @Override void write(StringBuilder out, Date value) { }
}

class NoClashOverride<T extends Date> extends Adapter<Object> {
    @Override void write(StringBuilder out, Object value) { }
    // write(T) erases to write(Date): an overload, not an override
    @Override void write(StringBuilder out, T value) { } // expect
}

class ClashOverride<T extends Date> extends Adapter<Date> {
    @Override void write(StringBuilder out, Date value) { }
    @Override void write(StringBuilder out, T value) { } // expect
}
