package lookalike;
import org.springframework.stereotype.Service;

// a project annotation named Mapper is not MapStruct: nothing is generated
@interface Mapper {}
@Mapper abstract class LookalikeMapper {}
@Service class UsesLookalike { UsesLookalike(LookalikeMapper m) {} } // expect

@org.springframework.boot.autoconfigure.SpringBootApplication class FixtureApp { public static void main(String[] args) {} }
