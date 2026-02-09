import { useGetPostsQuery } from "../state/api";

export function PostList() {
    const { data, isLoading } = useGetPostsQuery();
    if (isLoading) return <p>Loading...</p>;
    return (
        <ul>
            {data?.map((post: any) => <li key={post.id}>{post.title}</li>)}
        </ul>
    );
}
