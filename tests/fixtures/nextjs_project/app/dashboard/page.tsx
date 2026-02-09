import { PostList } from "../../src/components/PostList";
import { useAuth } from "../../src/hooks/useAuth";

export default function Page() {
    const { user } = useAuth();
    return (
        <main>
            <h1>Welcome {user}</h1>
            <PostList />
        </main>
    );
}
