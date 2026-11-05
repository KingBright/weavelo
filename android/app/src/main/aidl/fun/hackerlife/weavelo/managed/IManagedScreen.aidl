package fun.hackerlife.weavelo.managed;
import android.view.Surface;
/** Local UID-bound display operations; never a network or command endpoint. */
interface IManagedScreen {
    String info();
    int createMirror(int width, int height, int density);
    void setSurface(int id, in Surface surface);
    void resize(int id, int width, int height, int density);
    void release(int id);
}
