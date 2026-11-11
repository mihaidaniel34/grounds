import java.util.Collections;
import java.util.List;
import java.util.Objects;

// requireNonNullElse(Get) proves its RESULT non-null, not its argument
class RequireNonNullElse {
    void literalNull() {
        Object value = null;
        Objects.requireNonNullElse(value, "fallback");
        // reported as always null: the argument is not checked
        if (value == null) System.out.println("still null"); // expect
    }
    void argumentUnknown(Object value) {
        Objects.requireNonNullElse(value, "fallback");
        if (value == null) System.out.println("still null");
    }
    void resultIsNonNull(Object value) {
        Object v = Objects.requireNonNullElse(value, "fallback");
        if (v == null) { } // expect
        if (value == null) { }
        Object w = Objects.requireNonNullElseGet(value, Object::new);
        if (w == null) { } // expect
        if (value == null) { }
    }
    void requireNonNull(Object value) {
        Objects.requireNonNull(value, "value");
        if (value == null) { } // expect
    }
}

// membership in a collection never proves non-nullness: collections may hold null
class CollectionContains {
    void contains(Object value) {
        List<Object> allowed = Collections.singletonList(null);
        if (allowed.contains(value)) {
            if (value == null) System.out.println("contains null");
        }
    }
    void isEmpty(List<Object> values) {
        if (!values.isEmpty()) {
            // dereferenced by the call
            if (values == null) { } // expect
        }
    }
}

// a project method is not a library contract because of its name
class ProjectNamesake {
    static boolean contains(Object o) { return true; }
    static boolean isEmpty(Object o) { return false; }
    static boolean hasText(Object o) { return true; }
    static boolean nonNull(Object o) { return true; }
    boolean startsWith(Object o) { return true; }
    void f(Object value) {
        if (contains(value)) {
            if (value == null) System.out.println("null");
        }
        if (!isEmpty(value)) {
            if (value == null) System.out.println("null");
        }
        if (hasText(value)) {
            if (value == null) System.out.println("null");
        }
        if (nonNull(value)) {
            if (value == null) System.out.println("null");
        }
        if (this.startsWith(value)) {
            if (value == null) System.out.println("null");
        }
    }
}

// the JDK contracts that do hold, by owner
class JdkContracts {
    void f(String value) {
        if ("x".equals(value)) {
            if (value == null) { } // expect
        }
        if ("x".equalsIgnoreCase(value)) {
            if (value == null) { } // expect
        }
        if ("abc".contains(value)) {
            if (value == null) { } // expect
        }
        if ("abc".startsWith(value)) {
            if (value == null) { } // expect
        }
        if (Objects.nonNull(value)) {
            if (value == null) { } // expect
        }
    }
}

// JUnit 4 checks its LAST argument: the first is a message, which may be null
class JUnit4Message {
    void f(String message, Object value) {
        org.junit.Assert.assertNotNull(message, value);
        if (value == null) { } // expect
        if (message == null) { }
    }
}

// an `assertThat(..).isNotNull()` of the project is not AssertJ's
class ProjectAssertThat {
    static class Check { Check isNotNull() { return this; } }
    static Check assertThat(Object o) { return new Check(); }
    void f(Object value) {
        assertThat(value).isNotNull();
        if (value == null) { }
    }
    void g(Object value) {
        org.assertj.core.api.Assertions.assertThat(value).isNotNull();
        if (value == null) { } // expect
    }
}

// equals of a final JDK class is false for null; a project equals may not be
class EqualsByOwner {
    enum Color { RED }
    static class Loose { @Override public boolean equals(Object o) { return true; } }
    void f(Object value, Integer one, Loose loose) {
        if (Color.RED.equals(value)) {
            if (value == null) { } // expect
        }
        if (one.equals(value)) {
            if (value == null) { } // expect
        }
        if (loose.equals(value)) {
            if (value == null) { }
        }
    }
}

// a project `assertThat` returning the library's assertion for another value:
// the chain ends in AssertJ, but the argument is not its subject
class WrapperAssertThat {
    static org.assertj.core.api.ObjectAssert<Object> assertThat(Object value) {
        return org.assertj.core.api.Assertions.assertThat((Object) "fallback");
    }
    void f(Object value) {
        assertThat(value).isNotNull();
        if (value == null) { }
    }
}

// `requireNonNull` imported statically, in a class whose superclass we cannot
// see: that superclass may declare its own `requireNonNull`, which javac binds
class ShadowedImport extends ext.Base {
    void f(Object value) {
        Objects.requireNonNull(value);
        if (value == null) { } // expect
    }
}

// soft assertions collect failures instead of throwing: no contract
class Soft extends org.assertj.core.api.SoftAssertions {
    void f(Object value) {
        assertThat(value).isNotNull();
        if (value == null) { }
    }
}
