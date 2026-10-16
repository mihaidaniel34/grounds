package demo;

import java.util.List;
import java.util.Map.Entry;
import java.util.concurrent.*;
import static java.util.Map.entry;
import static java.util.Collections.*;
import java.util.NoSuchCollection; // expect
import java.util.ImmutableCollections; // expect
import java.util.nosuchpackage.*; // expect
import static java.util.NoSuchHelper.helper; // expect
import org.example.Missing;

class Imports {
    List<Entry<String, String>> pairs = List.of(entry("a", "b"));
}
