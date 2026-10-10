package app;
import org.springframework.stereotype.Service;
import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.beans.factory.annotation.Qualifier;
interface Store {}
@Service class StoreImpl implements Store {}
@Service class Consumer {
    // left unset at runtime: not an error
    @Autowired(required = false)
    @Qualifier("absent")
    Store optionalStore;

    @Autowired
    @Qualifier("absent")
    Store requiredStore; // expect
}
