package main

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
)

type issueData struct {
	State       string          `json:"state"`
	Association string          `json:"author_association"`
	Title       string          `json:"title"`
	Body        string          `json:"body"`
	PullRequest json.RawMessage `json:"pull_request"`
	Labels      []struct {
		Name string `json:"name"`
	} `json:"labels"`
}

func (issue issueData) hasLabel(name string) bool {
	for _, label := range issue.Labels {
		if label.Name == name {
			return true
		}
	}
	return false
}

func trusted(association string) bool {
	return association == "OWNER" || association == "MEMBER" || association == "COLLABORATOR"
}

func ghJSON(ctx context.Context, target any, args ...string) error {
	data, err := command(ctx, "", "gh", args...)
	if err != nil {
		return err
	}
	if err := json.Unmarshal(data, target); err != nil {
		return fmt.Errorf("invalid gh JSON: %w", err)
	}
	return nil
}

func readIssue(ctx context.Context, issue issueURL) (issueData, error) {
	var data issueData
	err := ghJSON(ctx, &data, "api", "repos/"+issue.repo+"/issues/"+issue.number)
	if err == nil && data.State != "open" && data.State != "closed" {
		err = fmt.Errorf("invalid issue state %q", data.State)
	}
	return data, err
}

func issueMarkdown(ctx context.Context, issue issueURL, data issueData) (string, error) {
	var pages [][]struct {
		Association string `json:"author_association"`
		User        *struct {
			Login string `json:"login"`
		} `json:"user"`
		Body string `json:"body"`
	}
	if err := ghJSON(ctx, &pages, "api", "repos/"+issue.repo+"/issues/"+issue.number+"/comments?per_page=100", "--paginate", "--slurp"); err != nil {
		return "", err
	}
	var markdown strings.Builder
	fmt.Fprintf(&markdown, "# %s\n\n%s\n\n%s\n", data.Title, issue.url, data.Body)
	for _, page := range pages {
		for _, comment := range page {
			markdown.WriteString("\n---\n\n")
			if trusted(comment.Association) && comment.User != nil && comment.User.Login != "" {
				fmt.Fprintf(&markdown, "Comment by @%s:\n\n%s\n", comment.User.Login, comment.Body)
			} else {
				markdown.WriteString("_A comment from an account that is not a Trusted account was left out._\n")
			}
		}
	}
	return markdown.String(), nil
}
