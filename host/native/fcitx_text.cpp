// Fcitx runs this addon in its input-method process. The Unix peer must be
// the installed Weavelo background owner. No text travels on the session bus.
#include <fcitx-utils/event.h>
#include <fcitx-utils/unixfd.h>
#include <fcitx-utils/utf8.h>
#include <fcitx/addonfactory.h>
#include <fcitx/addoninstance.h>
#include <fcitx/addonmanager.h>
#include <fcitx/inputcontext.h>
#include <fcitx/instance.h>

#include <array>
#include <cerrno>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <fcntl.h>
#include <pwd.h>
#include <string>
#include <string_view>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <time.h>
#include <unistd.h>

namespace {

constexpr size_t headerSize = 10;
constexpr size_t maxTextBytes = 1024;
constexpr uint64_t maxAgeNs = 250'000'000;

bool sameFile(const struct stat &a, const struct stat &b) {
    return a.st_dev == b.st_dev && a.st_ino == b.st_ino;
}

bool validText(std::string_view text) {
    if (text.empty() || text.size() > maxTextBytes || !fcitx::utf8::validate(text)) {
        return false;
    }
    for (size_t index = 0; index < text.size(); ++index) {
        const auto byte = static_cast<unsigned char>(text[index]);
        if ((byte < 0x20 && byte != '\r' && byte != '\n' && byte != '\t') || byte == 0x7f) {
            return false;
        }
        if (byte == 0xc2) {
            const auto next = static_cast<unsigned char>(text[index + 1]);
            if (next >= 0x80 && next <= 0x9f) {
                return false;
            }
        }
    }
    return true;
}

bool fresh(uint64_t expires) {
    struct timespec clock {};
    if (clock_gettime(CLOCK_MONOTONIC, &clock) != 0) {
        return false;
    }
    const uint64_t now = uint64_t(clock.tv_sec) * 1'000'000'000 + uint64_t(clock.tv_nsec);
    return expires > now && expires - now <= maxAgeNs;
}

class TextAddon final : public fcitx::AddonInstance {
public:
    explicit TextAddon(fcitx::Instance *instance) : instance_(instance) {}

    ~TextAddon() override {
        closeClient();
        struct stat current {};
        if (ownsSocket_ && lstat(path_.c_str(), &current) == 0 && sameFile(current, socketIdentity_)) {
            unlink(path_.c_str());
        }
    }

    bool start() {
        const char *runtime = std::getenv("XDG_RUNTIME_DIR");
        struct stat directory {};
        const auto *user = getpwuid(geteuid());
        if (!user || !runtime || runtime[0] != '/' || lstat(runtime, &directory) != 0 ||
            !S_ISDIR(directory.st_mode) || directory.st_uid != geteuid() || (directory.st_mode & 0077)) {
            return false;
        }
        executable_ = std::string(user->pw_dir) + "/.local/opt/weavelo/desktop/weavelo";
        path_ = std::string(runtime) + "/weavelo-fcitx-text-v1-" + std::to_string(getpid()) + ".sock";
        struct sockaddr_un address {};
        address.sun_family = AF_UNIX;
        if (path_.size() >= sizeof(address.sun_path)) {
            return false;
        }
        std::memcpy(address.sun_path, path_.c_str(), path_.size() + 1);
        listener_ = fcitx::UnixFD::own(socket(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC, 0));
        if (listener_.fd() < 0 || bind(listener_.fd(), reinterpret_cast<sockaddr *>(&address),
                                     offsetof(sockaddr_un, sun_path) + path_.size() + 1) != 0) {
            return false;
        }
        ownsSocket_ = lstat(path_.c_str(), &socketIdentity_) == 0;
        if (!ownsSocket_ || chmod(path_.c_str(), 0600) != 0 || listen(listener_.fd(), 4) != 0) {
            return false;
        }
        const auto invalidate = [this](fcitx::Event &) { ++focusGeneration_; };
        focusIn_ = instance_->watchEvent(fcitx::EventType::InputContextFocusIn,
                                         fcitx::EventWatcherPhase::PreInputMethod, invalidate);
        focusOut_ = instance_->watchEvent(fcitx::EventType::InputContextFocusOut,
                                          fcitx::EventWatcherPhase::PreInputMethod, invalidate);
        listenerEvent_ = instance_->eventLoop().addIOEvent(listener_.fd(), fcitx::IOEventFlag::In,
            [this](fcitx::EventSourceIO *, int fd, fcitx::IOEventFlags) {
                acceptClient(fd);
                return true;
            });
        return bool(listenerEvent_);
    }

private:
    bool productPeer(int fd) const {
        struct ucred peer {};
        socklen_t size = sizeof(peer);
        if (getsockopt(fd, SOL_SOCKET, SO_PEERCRED, &peer, &size) != 0 ||
            size != sizeof(peer) || peer.uid != geteuid() || peer.pid <= 1) {
            return false;
        }
        const auto proc = "/proc/" + std::to_string(peer.pid);
        struct stat expected {}, actual {};
        if (lstat(executable_.c_str(), &expected) != 0 || !S_ISREG(expected.st_mode) ||
            expected.st_uid != peer.uid || (expected.st_mode & 0022) ||
            stat((proc + "/exe").c_str(), &actual) != 0 || !sameFile(expected, actual)) {
            return false;
        }
        auto command = fcitx::UnixFD::own(open((proc + "/cmdline").c_str(), O_RDONLY | O_CLOEXEC));
        std::array<char, 1024> bytes {};
        const ssize_t length = read(command.fd(), bytes.data(), bytes.size());
        if (length <= 0 || size_t(length) == bytes.size()) {
            return false;
        }
        const std::string_view arguments(bytes.data(), size_t(length));
        const size_t first = arguments.find('\0');
        constexpr char role[] = "--background-service";
        return first != std::string_view::npos &&
            arguments.substr(first + 1) == std::string_view(role, sizeof(role));
    }

    void acceptClient(int fd) {
        auto peer = fcitx::UnixFD::own(accept4(fd, nullptr, nullptr, SOCK_NONBLOCK | SOCK_CLOEXEC));
        if (peer.fd() < 0 || !productPeer(peer.fd())) {
            return;
        }
        auto *context = instance_->mostRecentInputContext();
        if (!context || !context->hasFocus()) {
            const char rejected = 0;
            send(peer.fd(), &rejected, 1, MSG_NOSIGNAL);
            return;
        }
        closeClient();
        clientEvent_.reset();
        client_ = std::move(peer);
        context_ = context->watch();
        preparedGeneration_ = focusGeneration_;
        used_ = 0;
        clientEvent_ = instance_->eventLoop().addIOEvent(client_.fd(), fcitx::IOEventFlag::In,
            [this](fcitx::EventSourceIO *, int client, fcitx::IOEventFlags) {
                readClient(client);
                return true;
            });
        const char ready = 1;
        if (!clientEvent_ || send(client_.fd(), &ready, 1, MSG_NOSIGNAL) != 1) {
            closeClient();
        }
    }

    void closeClient() {
        if (clientEvent_) {
            clientEvent_->setEnabled(false);
        }
        client_.reset();
        context_.unwatch();
        // The buffer may have held a password or other private input.
        explicit_bzero(buffer_.data(), buffer_.size());
    }

    void readClient(int fd) {
        while (used_ < buffer_.size()) {
            const ssize_t count = recv(fd, buffer_.data() + used_, buffer_.size() - used_, 0);
            if (count > 0) {
                used_ += size_t(count);
            } else if (count == 0) {
                const char accepted = commit() ? 1 : 0;
                send(fd, &accepted, 1, MSG_NOSIGNAL);
                break;
            } else if (errno == EAGAIN || errno == EWOULDBLOCK) {
                return;
            } else if (errno != EINTR) {
                break;
            }
        }
        closeClient();
    }

    bool commit() {
        if (used_ <= headerSize) {
            return false;
        }
        uint64_t expires = 0;
        for (size_t byte = 0; byte < 8; ++byte) {
            expires |= uint64_t(buffer_[byte]) << (8 * byte);
        }
        const size_t length = buffer_[8] | (size_t(buffer_[9]) << 8);
        const std::string_view text(reinterpret_cast<const char *>(buffer_.data() + headerSize),
                                    used_ - headerSize);
        auto *context = context_.get();
        if (length != text.size() || !validText(text) || !context || !context->hasFocus() ||
            context != instance_->mostRecentInputContext() || preparedGeneration_ != focusGeneration_ ||
            !fresh(expires)) {
            return false;
        }
        context->commitString(std::string(text));
        return true;
    }

    fcitx::Instance *instance_;
    std::string executable_;
    std::string path_;
    struct stat socketIdentity_ {};
    bool ownsSocket_ = false;
    uint64_t focusGeneration_ = 0;
    uint64_t preparedGeneration_ = 0;
    fcitx::TrackableObjectReference<fcitx::InputContext> context_;
    std::array<unsigned char, headerSize + maxTextBytes + 1> buffer_ {};
    size_t used_ = 0;
    fcitx::UnixFD listener_;
    fcitx::UnixFD client_;
    std::unique_ptr<fcitx::HandlerTableEntry<fcitx::EventHandler>> focusIn_;
    std::unique_ptr<fcitx::HandlerTableEntry<fcitx::EventHandler>> focusOut_;
    std::unique_ptr<fcitx::EventSourceIO> listenerEvent_;
    std::unique_ptr<fcitx::EventSourceIO> clientEvent_;
};

class Factory final : public fcitx::AddonFactory {
    fcitx::AddonInstance *create(fcitx::AddonManager *manager) override {
        auto addon = std::make_unique<TextAddon>(manager->instance());
        return addon->start() ? addon.release() : nullptr;
    }
};

} // namespace

FCITX_ADDON_FACTORY(Factory)
