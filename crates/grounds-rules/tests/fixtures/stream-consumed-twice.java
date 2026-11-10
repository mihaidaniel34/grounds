import java.util.List;
import java.util.function.Supplier;
import java.util.stream.Collectors;
import java.util.stream.IntStream;
import java.util.stream.Stream;

class A {
    long twice(List<String> l) {
        Stream<String> s = l.stream();
        s.count();
        return s.count(); // expect
    }

    List<String> droppedIntermediate(List<String> l) {
        Stream<String> s = l.stream();
        s.filter(x -> !x.isEmpty());
        return s.collect(Collectors.toList()); // expect
    }

    int primitive(int[] a) {
        IntStream s = IntStream.of(a);
        int max = s.max().orElse(0);
        int min = s.min().orElse(0); // expect
        return max - min;
    }

    long param(Stream<String> s, boolean log) {
        if (log) {
            s.forEach(System.out::println);
        }
        return s.count(); // expect
    }

    long sameExpression(Stream<String> s) {
        return s.count() + s.distinct().count(); // expect
    }

    long withVar(List<String> l) {
        var s = l.stream();
        long n = s.count();
        return n + s.count(); // expect
    }

    Object exclusive(Stream<String> s, int mode, boolean c) {
        if (c) {
            return s.count();
        } else if (mode == 3) {
            return s.findFirst();
        }
        switch (mode) {
            case 0:
                return s.collect(Collectors.toList());
            case 1:
                return s.count();
            default:
                break;
        }
        return c ? s.findAny() : s.findFirst();
    }

    long earlyExit(Stream<String> s, boolean c) {
        if (c) {
            return s.count();
        }
        return s.distinct().count();
    }

    long reassigned(List<String> l, boolean c) {
        Stream<String> s = l.stream();
        if (c) {
            s = s.filter(x -> !x.isEmpty());
        }
        s = s.map(String::trim);
        return s.count();
    }

    long configured(Stream<String> s) {
        s.onClose(() -> System.out.println("closed"));
        s.parallel();
        long n = s.count();
        s.close();
        return n;
    }

    long perIteration(List<List<String>> ls) {
        long n = 0;
        for (List<String> l : ls) {
            Stream<String> s = l.stream();
            n += s.count();
        }
        return n;
    }

    long supplier(Supplier<Stream<String>> sup) {
        return sup.get().count() + sup.get().count();
    }

    long fallback(Stream<String> s) {
        try {
            return s.count();
        } catch (IllegalStateException e) {
            return s.distinct().count();
        }
    }
}
